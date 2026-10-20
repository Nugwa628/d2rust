//! The map's tile art for the play preview: the DT1 tiles the near rooms'
//! records name (the entry identity: path and tile
//! index), read from the user's archives on demand and made resident in
//! [`ViewAssets::frames`] (one `FramePart::Tile` set per entry),
//! with each tile's blocks for the wall culling of camera;
//! and the act PL2's palette-table block pushed into
//! [`ViewAssets::maps`] once per act, so shadow tiles and translucent
//! walls have their blend tables (G10).
//!
//! The archive name of a record's path is the server's
//! (`d2_server::world_data::archive_name`: relative to
//! `DATA\GLOBAL\TILES\`). A file no archive holds, one that does not parse,
//! a tile index past the file's tiles or a tile without blocks is
//! remembered as a failure of that entry, never retried; what the preview
//! does with it is [`super::preview`]'s (skip the item, log it once).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use d2_formats::dt1::Dt1;
use d2_formats::palette::Pl2;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::frames::{FramePart, FrameSet, FrameSetKey};
use crate::rules::shading::ShadeTables;
use crate::rules::BlockRect;
use crate::scene::MapTable;

use super::near_rooms::Dt1Entry;
use super::ViewAssets;

/// The frame-set key of a record's DT1 entry: the canonical archive path
/// of `path` and the tile `index`.
pub fn tile_key(path: &[u8], index: u32) -> Result<FrameSetKey, String> {
    let archive = archive_name(path);
    let canonical = CanonicalPath::new(&archive).map_err(|e| format!("{archive}: {e}"))?;
    FrameSetKey::new(canonical.as_str(), FramePart::Tile(index))
        .map_err(|e| format!("{archive}: {e}"))
}

/// The archive name of a record's DT1 path.
pub fn archive_name(path: &[u8]) -> String {
    String::from_utf8_lossy(&d2_server::world_data::archive_name(path)).into_owned()
}

/// The DT1 tiles and act shade tables of the preview map.
#[derive(Clone, Default)]
pub struct TileAssets {
    /// The user's archives; `None` (synthetic data): every entry fails.
    source: Option<Arc<dyn FileSource>>,
    /// Parsed DT1 files by archive name, or why the file cannot be read.
    files: BTreeMap<String, Result<Arc<Dt1>, String>>,
    /// Resident entries and their blocks.
    blocks: BTreeMap<FrameSetKey, Vec<BlockRect>>,
    /// The grid coordinates (block record bytes 6, 7) of each resident
    /// entry's blocks, for the floor light grid.
    grids: BTreeMap<FrameSetKey, Vec<(u8, u8)>>,
    /// Entries that cannot be made resident, and why.
    failed: BTreeMap<FrameSetKey, String>,
    /// The `pal.pl2` of acts 0…4, when known.
    pl2: Option<Arc<[Vec<u8>; 5]>>,
    /// Each act's tables once pushed into the frame's map table.
    shades: [Option<ShadeTables>; 5],
}

impl fmt::Debug for TileAssets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TileAssets")
            .field("source", &self.source.is_some())
            .field("files", &self.files.len())
            .field("resident", &self.blocks.len())
            .field("failed", &self.failed.len())
            .field("shades", &self.shades)
            .finish()
    }
}

impl TileAssets {
    /// Tiles from `source`, with the five act palettes `pl2` (act `a` at
    /// index `a`) for the shade tables.
    pub fn new(source: Option<Arc<dyn FileSource>>, pl2: Option<[Vec<u8>; 5]>) -> Self {
        TileAssets {
            source,
            pl2: pl2.map(Arc::new),
            ..TileAssets::default()
        }
    }

    /// The archives the tiles are read from.
    pub fn source(&self) -> Option<Arc<dyn FileSource>> {
        self.source.clone()
    }

    /// Makes every entry resident that is neither resident nor failed.
    /// Returns the failures this call found (each entry once).
    pub fn ensure<'a>(
        &mut self,
        entries: impl IntoIterator<Item = &'a Dt1Entry>,
        assets: &mut ViewAssets,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for (path, index) in entries {
            let key = match tile_key(path, *index) {
                Ok(k) => k,
                Err(m) => {
                    bevy::log::warn!(target: "d2_asset_fallback", "{m}");
                    found.push(m);
                    continue;
                }
            };
            if self.blocks.contains_key(&key) || self.failed.contains_key(&key) {
                continue;
            }
            let loaded = self
                .load(path, *index)
                .and_then(|(set, blocks, grids, id)| {
                    assets.tile_ids.insert(key.clone(), id);
                    assets
                        .frames
                        .insert(key.clone(), set)
                        .map(|_| (blocks, grids))
                        .map_err(|e| e.to_string())
                });
            match loaded {
                Ok((blocks, grids)) => {
                    self.grids.insert(key.clone(), grids);
                    self.blocks.insert(key, blocks);
                }
                Err(m) => {
                    // A parsed blank tile has no image to draw. It is not
                    // a missing asset, and synthetic maps use these tiles.
                    if m != "the tile has no blocks" {
                        bevy::log::warn!(target: "d2_asset_fallback", asset = %archive_name(path), "{m}");
                    }
                    let m = format!("{} tile {index}: {m}", archive_name(path));
                    found.push(m.clone());
                    self.failed.insert(key, m);
                }
            }
        }
        found
    }

    /// The grid coordinates of a resident entry's blocks, in block order.
    pub fn grids(&self, key: &FrameSetKey) -> Option<&[(u8, u8)]> {
        self.grids.get(key).map(Vec::as_slice)
    }

    /// The blocks of a resident entry; `None` when it is not resident.
    pub fn blocks(&self, key: &FrameSetKey) -> Option<&[BlockRect]> {
        self.blocks.get(key).map(Vec::as_slice)
    }

    /// Why an entry is not resident, if it failed.
    pub fn failure(&self, key: &FrameSetKey) -> Option<&str> {
        self.failed.get(key).map(String::as_str)
    }

    /// Pushes act `act`'s palette-table block into `maps` once;
    /// `act` past 4 is act 0's.
    /// Without palettes, nothing.
    pub fn ensure_shades(&mut self, act: u8, maps: &mut MapTable) -> Result<(), String> {
        let a = if act < 5 { usize::from(act) } else { 0 };
        if self.shades[a].is_some() {
            return Ok(());
        }
        let Some(pl2) = &self.pl2 else {
            return Ok(());
        };
        let parsed = Pl2::parse(&pl2[a]).map_err(|e| format!("act {a} pal.pl2: {e}"))?;
        self.shades[a] = Some(ShadeTables::push(maps, &parsed));
        Ok(())
    }

    /// Act `act`'s tables, once pushed.
    pub fn shades(&self, act: u8) -> Option<&ShadeTables> {
        let a = if act < 5 { usize::from(act) } else { 0 };
        self.shades[a].as_ref()
    }

    /// The frame set and blocks of tile `index` of `path`.
    fn load(&mut self, path: &[u8], index: u32) -> LoadedTile {
        let archive = archive_name(path);
        let file = match self.files.get(&archive) {
            Some(f) => f.clone(),
            None => {
                let f = read_dt1(self.source.as_deref(), &archive);
                self.files.insert(archive, f.clone());
                f
            }
        }?;
        let tile = file
            .tiles
            .get(index as usize)
            .ok_or_else(|| format!("past the file's {} tiles", file.tiles.len()))?;
        let set = FrameSet::from_dt1(&file, index).map_err(|e| e.to_string())?;
        if set.frames.is_empty() {
            return Err("the tile has no blocks".into());
        }
        let grids = tile.blocks.iter().map(|b| (b.grid_x, b.grid_y)).collect();
        let id = [
            tile.orientation,
            tile.main_index,
            tile.sub_index,
            tile.rarity,
        ];
        Ok((set, BlockRect::of_tile(tile), grids, id))
    }
}

/// A loaded tile: its frame set, blocks, block grid coordinates and header
/// `orientation.main.sub.rarity`.
type LoadedTile = Result<(FrameSet, Vec<BlockRect>, Vec<(u8, u8)>, [u32; 4]), String>;

fn read_dt1(source: Option<&dyn FileSource>, archive: &str) -> Result<Arc<Dt1>, String> {
    let source = source.ok_or("no game archives (synthetic data)")?;
    crate::assets::path::read_dt1_file(source, archive)
        .ok_or_else(|| "in no archive".to_string())?
        .map(Arc::new)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::assets::path::MemorySource;

    /// A DT1 of one tile with one 32 × 32 RLE block:
    /// the first row starts with pixels 0x0A, 0x0B.
    pub(crate) fn dt1_bytes() -> Vec<u8> {
        let encoded = [0x00u8, 0x02, 0x0A, 0x0B];
        let mut d = Vec::new();
        d.extend_from_slice(&7u32.to_le_bytes());
        d.extend_from_slice(&6u32.to_le_bytes());
        d.extend_from_slice(&[0; 260]);
        d.extend_from_slice(&1u32.to_le_bytes()); // tiles
        d.extend_from_slice(&276u32.to_le_bytes()); // first tile
        let mut tile = vec![0u8; 96];
        tile[0x48..0x4C].copy_from_slice(&372u32.to_le_bytes());
        tile[0x50..0x54].copy_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&tile);
        for v in [0u16, 0, 0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&0x1001u16.to_le_bytes());
        d.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&20u32.to_le_bytes());
        d.extend_from_slice(&encoded);
        d
    }

    pub(crate) fn source() -> Arc<dyn FileSource> {
        let mut m = MemorySource::default();
        m.insert(r"DATA\GLOBAL\TILES\ACT1\TOWN\floor.dt1", dt1_bytes());
        m.insert(r"DATA\GLOBAL\TILES\ACT1\TOWN\bad.dt1", vec![1, 2, 3]);
        Arc::new(m)
    }

    fn assets() -> ViewAssets {
        ViewAssets::new(crate::app::play::unspecified_palette())
    }

    #[test]
    fn tile_keys_are_the_canonical_archive_path() {
        let k = tile_key(b"Act1/Town/Floor.dt1", 3).unwrap();
        assert_eq!(k.path(), "data/global/tiles/act1/town/floor.dt1");
        assert_eq!(k.part(), FramePart::Tile(3));
        // A path that already carries the prefix is the name.
        assert_eq!(
            tile_key(br"Data\Global\Tiles\ACT1\TOWN\floor.dt1", 3).unwrap(),
            k
        );
    }

    #[test]
    fn blank_tiles_are_quiet_but_missing_files_are_captured() {
        let mut blank = dt1_bytes();
        blank[276 + 0x50..276 + 0x54].fill(0);
        blank.truncate(372);
        let mut source = MemorySource::default();
        source.insert(r"DATA\GLOBAL\TILES\ACT1\TOWN\blank.dt1", blank);
        let mut tiles = TileAssets::new(Some(Arc::new(source)), None);
        let mut assets = assets();
        let capture = crate::app::checks::capture::Capture::default();
        bevy::log::tracing::dispatcher::with_default(&capture.dispatch(), || {
            let blank = (b"Act1/Town/blank.dt1".to_vec(), 0);
            assert!(tiles.ensure([&blank], &mut assets)[0].contains("no blocks"));
            assert!(capture.take().is_empty());
            let missing = (b"Act1/Town/missing.dt1".to_vec(), 0);
            assert!(tiles.ensure([&missing], &mut assets)[0].contains("in no archive"));
        });
        assert_eq!(capture.take()[0].kind, "missing-asset");
    }

    #[test]
    fn entries_load_once_and_failures_are_kept() {
        let mut t = TileAssets::new(Some(source()), None);
        let mut a = assets();
        let good: Dt1Entry = (b"Act1/Town/floor.dt1".to_vec(), 0);
        let past: Dt1Entry = (b"Act1/Town/floor.dt1".to_vec(), 1);
        let missing: Dt1Entry = (b"Act1/Town/none.dt1".to_vec(), 0);
        let bad: Dt1Entry = (b"Act1/Town/bad.dt1".to_vec(), 0);
        let found = t.ensure([&good, &past, &missing, &bad, &good], &mut a);
        assert_eq!(found.len(), 3, "{found:?}");
        let key = tile_key(&good.0, 0).unwrap();
        assert!(a.frames.contains(&key));
        let blocks = t.blocks(&key).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].width, blocks[0].height), (32, 32));
        assert!(t
            .failure(&tile_key(&past.0, 1).unwrap())
            .unwrap()
            .contains("past the file's 1 tiles"));
        assert!(t
            .failure(&tile_key(&missing.0, 0).unwrap())
            .unwrap()
            .contains("in no archive"));
        assert!(t.failure(&tile_key(&bad.0, 0).unwrap()).is_some());
        // Nothing is retried or loaded twice.
        assert!(t.ensure([&good, &past, &missing], &mut a).is_empty());
    }

    #[test]
    fn without_archives_every_entry_fails() {
        let mut t = TileAssets::new(None, None);
        let mut a = assets();
        let e: Dt1Entry = (b"Act1/Town/floor.dt1".to_vec(), 0);
        let found = t.ensure([&e], &mut a);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("synthetic"), "{found:?}");
    }

    /// A `pal.pl2` of zeros: the fixed block only.
    pub(crate) fn pl2() -> Vec<u8> {
        vec![0; 1024 + 1714 * 256]
    }

    #[test]
    fn shade_tables_are_pushed_once_per_act() {
        let mut maps = MapTable::new();
        let mut t = TileAssets::new(None, None);
        t.ensure_shades(0, &mut maps).unwrap();
        assert!(t.shades(0).is_none(), "no palettes: no tables");
        let mut t = TileAssets::new(None, Some(std::array::from_fn(|_| pl2())));
        t.ensure_shades(1, &mut maps).unwrap();
        let first = *t.shades(1).unwrap();
        t.ensure_shades(1, &mut maps).unwrap();
        assert_eq!(*t.shades(1).unwrap(), first);
        assert!(t.shades(0).is_none());
        // Past act 4: act 0's.
        t.ensure_shades(9, &mut maps).unwrap();
        assert!(t.shades(0).is_some());
        let mut short = TileAssets::new(None, Some(std::array::from_fn(|_| vec![0; 10])));
        assert!(short.ensure_shades(0, &mut maps).is_err());
    }
}

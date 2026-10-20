//! The automap files next to the save: the `.map` index of four act
//! slots and the `.ma0`–`.ma3` data files of per-layer records.
//!
//! The byte logic works on in-memory buffers ([`open_index`], [`MaFile`]);
//! [`open_files`] and [`MaFile::write_to`] are the thin `std::fs` edge.

use std::path::{Path, PathBuf};

use super::cells::{Cell, TreeKind};
use super::AutomapError;

/// `.map` version.
pub const MAP_VERSION: u32 = 0xC;
/// `.map` size: version, next slot, four slot keys.
pub const MAP_BYTES: usize = 0x18;
/// Slots of the index (`.ma0`–`.ma3`).
pub const SLOTS: usize = 4;
/// Entries of a data file's offset table (indexed by layer id).
pub const TABLE_ENTRIES: usize = 100;
pub const TABLE_BYTES: usize = TABLE_ENTRIES * 4;
/// A record header: eight u32.
pub const RECORD_HEADER: usize = 32;
/// One stored cell: (cel, x, y) i16.
pub const TRIPLE: usize = 6;

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn bad(detail: String) -> AutomapError {
    AutomapError::File(detail)
}

/// The result of opening the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOpen {
    /// The slot k whose `.ma<k>` is the open data file.
    pub slot: usize,
    /// The header to write back (`None` when the key was found).
    pub rewrite: Option<[u8; MAP_BYTES]>,
    /// `.ma<k>` files to delete before opening.
    pub delete: Vec<usize>,
}

/// Open the `.map` index `file` (`None`: no file) for act key `key`
/// (the first u32 of the act record, S→C 0x03 u32@2).
pub fn open_index(file: Option<&[u8]>, key: u32) -> Result<IndexOpen, AutomapError> {
    let valid = file.filter(|b| b.len() >= MAP_BYTES && u32_at(b, 0) == Some(MAP_VERSION));
    let Some(b) = valid else {
        // Invalid: version 0xC, next 1, slot 0:= key, all four deleted.
        let mut h = [0u8; MAP_BYTES];
        put_u32(&mut h, 0, MAP_VERSION);
        put_u32(&mut h, 4, 1);
        put_u32(&mut h, 8, key);
        return Ok(IndexOpen {
            slot: 0,
            rewrite: Some(h),
            delete: (0..SLOTS).collect(),
        });
    };
    let slots: Vec<u32> = (0..SLOTS)
        .map(|k| u32_at(b, 8 + 4 * k).unwrap_or(0))
        .collect();
    if let Some(k) = slots.iter().position(|&s| s == key) {
        return Ok(IndexOpen {
            slot: k,
            rewrite: None,
            delete: Vec::new(),
        });
    }
    let next = u32_at(b, 4).unwrap_or(0);
    let k = next as usize;
    if k >= SLOTS {
        // The game indexes the slot array with it unchecked.
        return Err(bad(format!(".map next slot {next} is not 0–3 (§7 r1)")));
    }
    let mut h = [0u8; MAP_BYTES];
    h.copy_from_slice(&b[..MAP_BYTES]);
    put_u32(&mut h, 8 + 4 * k, key);
    put_u32(&mut h, 4, (next + 1) % SLOTS as u32);
    Ok(IndexOpen {
        slot: k,
        rewrite: Some(h),
        delete: vec![k],
    })
}

/// One record of a data file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaRecord {
    pub layer: u32,
    pub town_kind: u32,
    /// The act record's second u32 (S→C 0x03 u32@8).
    pub act2: u32,
    /// Floor, wall, unit, town blobs.
    pub blobs: [Vec<Cell>; 4],
}

/// The cel counts a load checks stored cels against: `maxi` for floors, walls
/// and units, `town[kind]` for the town blob (kind 0 draws with `maxi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CelLimits {
    pub maxi: u32,
    pub town: [u32; 4],
}

impl CelLimits {
    fn of(&self, tree: TreeKind, town_kind: u32) -> u32 {
        match tree {
            TreeKind::Town => self.town.get(town_kind as usize).copied().unwrap_or(0),
            _ => self.maxi,
        }
    }
}

/// What a load restored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    /// Cells per tree, saved flag 1, in file order.
    pub trees: [Vec<Cell>; 4],
    /// Whether the load cut the layer's chain.
    pub cut: bool,
}

/// A `.ma<k>` data file in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaFile {
    pub bytes: Vec<u8>,
}

impl Default for MaFile {
    fn default() -> Self {
        MaFile {
            bytes: vec![0; TABLE_BYTES],
        }
    }
}

impl MaFile {
    /// A data file from its bytes: empty → a fresh table.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, AutomapError> {
        if bytes.is_empty() {
            return Ok(MaFile::default());
        }
        if bytes.len() < TABLE_BYTES {
            return Err(bad(format!(
                ".ma file of {} bytes is shorter than its {TABLE_BYTES}-byte table (§7 r2)",
                bytes.len()
            )));
        }
        Ok(MaFile { bytes })
    }

    fn table_slot(layer: u32) -> Result<usize, AutomapError> {
        if (layer as usize) < TABLE_ENTRIES {
            Ok(4 * layer as usize)
        } else {
            Err(bad(format!(
                "layer {layer} past the {TABLE_ENTRIES}-entry table (§7 r2)"
            )))
        }
    }

    /// The chain of `layer`: (position of the pointer, record offset) per
    /// record, in order.
    fn chain(&self, layer: u32) -> Result<Vec<(usize, usize)>, AutomapError> {
        let mut at = Self::table_slot(layer)?;
        let mut out = Vec::new();
        loop {
            let off = u32_at(&self.bytes, at)
                .ok_or_else(|| bad(format!("link at {at} past the file")))?
                as usize;
            if off == 0 {
                return Ok(out);
            }
            if off + RECORD_HEADER > self.bytes.len() || out.iter().any(|&(_, o)| o == off) {
                return Err(bad(format!(
                    "record offset {off:#x} past the file or looping (§7 r3)"
                )));
            }
            out.push((at, off));
            // The link is the record's first u32.
            at = off;
        }
    }

    /// Appends `r` and links it from the end of its layer's chain
    /// (first record: the table entry). Returns its offset.
    pub fn append(&mut self, r: &MaRecord) -> Result<u32, AutomapError> {
        let chain = self.chain(r.layer)?;
        let off = self.bytes.len();
        let off32 = u32::try_from(off).map_err(|_| bad("file past 4 GiB".into()))?;
        let size = |b: &Vec<Cell>| (b.len() * TRIPLE) as u32;
        // Header = link, layer, town kind, act u32, floor / wall /
        // unit / town sizes.
        let header = [
            0,
            r.layer,
            r.town_kind,
            r.act2,
            size(&r.blobs[0]),
            size(&r.blobs[1]),
            size(&r.blobs[2]),
            size(&r.blobs[3]),
        ];
        for v in header {
            self.bytes.extend_from_slice(&v.to_le_bytes());
        }
        for blob in &r.blobs {
            for c in blob {
                for v in [c.cel, c.x, c.y] {
                    self.bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        // The table entry holds the first record's offset, each
        // record's link (its first u32) the next one's.
        let link_at = match chain.last() {
            None => Self::table_slot(r.layer)?,
            Some(&(_, last)) => last,
        };
        put_u32(&mut self.bytes, link_at, off32);
        Ok(off32)
    }

    /// The cells of layer `layer`, following its chain. `act2` is
    /// the current act record's second u32 (S→C 0x03 u32@8).
    pub fn load(
        &mut self,
        layer: u32,
        act2: u32,
        limits: CelLimits,
    ) -> Result<Loaded, AutomapError> {
        let chain = self.chain(layer)?;
        let mut out = Loaded::default();
        for (link, off) in chain {
            let h: Vec<u32> = (0..8)
                .map(|i| u32_at(&self.bytes, off + 4 * i).unwrap_or(0))
                .collect();
            // A record of another layer stops the load and clears the
            // chain at that point.
            if h[1] != layer {
                put_u32(&mut self.bytes, link, 0);
                out.cut = true;
                return Ok(out);
            }
            let town_kind = h[2];
            let mut at = off + RECORD_HEADER;
            let mut cut = false;
            for (i, tree) in TreeKind::ALL.into_iter().enumerate() {
                let size = h[4 + i] as usize;
                let end = at
                    .checked_add(size)
                    .filter(|&e| e <= self.bytes.len() && size.is_multiple_of(TRIPLE))
                    .ok_or_else(|| {
                        bad(format!(
                            "blob of {size} bytes at {at:#x} past the file (§7 r2)"
                        ))
                    })?;
                // The unit blob only for the same act record.
                if tree == TreeKind::Unit && h[3] != act2 {
                    at = end;
                    continue;
                }
                let limit = limits.of(tree, town_kind);
                for t in self.bytes[at..end].as_chunks::<TRIPLE>().0 {
                    let v = |k: usize| i16::from_le_bytes([t[2 * k], t[2 * k + 1]]);
                    let (cel, x, y) = (v(0), v(1), v(2));
                    if i64::from(cel) >= i64::from(limit) {
                        // Reading: the blob stops and
                        // the chain is cut where this record is linked,
                        // like a layer mismatch; the rest of the record
                        // still loads and no later record is read.
                        cut = true;
                        break;
                    }
                    out.trees[i].push(Cell {
                        saved: true,
                        cel,
                        x,
                        y,
                    });
                }
                at = end;
            }
            if cut {
                put_u32(&mut self.bytes, link, 0);
                out.cut = true;
                return Ok(out);
            }
        }
        Ok(out)
    }

    /// Writes the file to `path`.
    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, &self.bytes)
    }
}

/// The automap file paths of character `name` in `dir`:
/// `<dir>\<name>.map` and `<name>.ma0`–`.ma3`.
pub fn paths(dir: &Path, name: &str) -> (PathBuf, [PathBuf; SLOTS]) {
    (
        dir.join(format!("{name}.map")),
        std::array::from_fn(|k| dir.join(format!("{name}.ma{k}"))),
    )
}

/// On disk: opens (and rewrites) the index for act key `key`,
/// deletes the slots it names and reads the data file. Returns the data
/// file, its path and its slot.
pub fn open_files(
    dir: &Path,
    name: &str,
    key: u32,
) -> Result<(MaFile, PathBuf, usize), AutomapError> {
    let io = |what: &Path, e: std::io::Error| bad(format!("{}: {e}", what.display()));
    let (map, ma) = paths(dir, name);
    // A first start has no save folder yet; the files are made in it.
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let index = match std::fs::read(&map) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(&map, e)),
    };
    let open = open_index(index.as_deref(), key)?;
    for &k in &open.delete {
        match std::fs::remove_file(&ma[k]) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(&ma[k], e)),
        }
    }
    if let Some(h) = open.rewrite {
        std::fs::write(&map, h).map_err(|e| io(&map, e))?;
    }
    let path = ma[open.slot].clone();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The handle is opened with OPEN_ALWAYS, so an empty
            // `.ma<k>` exists from the open on.
            std::fs::File::create(&path).map_err(|e| io(&path, e))?;
            Vec::new()
        }
        Err(e) => return Err(io(&path, e)),
    };
    Ok((MaFile::from_bytes(bytes)?, path, open.slot))
}

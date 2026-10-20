//! The cell picker over the converted `automap.txt`
//! records and level-name ranges.

use d2_data::fixup::maps::Automap as AutomapMaps;
use d2_sim::rng::Seed;

use super::AutomapError;

/// One converted record (0x20 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PickRecord {
    /// +0x00: LevelName as a list-A index.
    pub level: i32,
    /// +0x04: TileName as a list-B index.
    pub tile: i32,
    /// +0x08..+0x0A.
    pub style: u8,
    pub start: u8,
    pub end: u8,
    /// +0x0C: Cel1–Cel4.
    pub cels: [i32; 4],
    /// +0x1C: cels before the first −1.
    pub count: i32,
}

fn i32_at(r: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]])
}

impl PickRecord {
    pub fn decode(r: &[u8; 0x20]) -> Self {
        PickRecord {
            level: i32_at(r, 0x00),
            tile: i32_at(r, 0x04),
            style: r[0x08],
            start: r[0x09],
            end: r[0x0A],
            cels: [
                i32_at(r, 0x0C),
                i32_at(r, 0x10),
                i32_at(r, 0x14),
                i32_at(r, 0x18),
            ],
            count: i32_at(r, 0x1C),
        }
    }
}

/// The picker's tables: the converted records and the 36 (first, end)
/// ranges indexed by list-A index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CelPicker {
    pub records: Vec<PickRecord>,
    pub ranges: Vec<(i32, i32)>,
}

/// The tile arguments of a pick: T = leveldefs `LevelType`, the DT1
/// tile's orientation o and its main and sub indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PickKey {
    pub level_type: u32,
    pub orientation: u32,
    pub main: u32,
    pub sub: u32,
}

impl CelPicker {
    pub fn new(maps: &AutomapMaps) -> Self {
        CelPicker {
            records: maps.records.iter().map(PickRecord::decode).collect(),
            ranges: maps.ranges.clone(),
        }
    }

    /// R1–r3: the cel of a tile, or −1. A winner draws one
    /// `roll(count)` on the automap seed (count 1 still draws; Randomness).
    pub fn pick(&self, k: PickKey, seed: &mut Seed) -> Result<i32, AutomapError> {
        // r1: T = 0 or first = −1 → −1.
        if k.level_type == 0 {
            return Ok(-1);
        }
        let &(first, end) = self.ranges.get(k.level_type as usize).ok_or_else(|| {
            AutomapError::Unresolved(format!(
                "§2 r1: LevelType {} past the {} level-name ranges",
                k.level_type,
                self.ranges.len()
            ))
        })?;
        if first == -1 {
            return Ok(-1);
        }
        if end == -1 {
            return Err(AutomapError::fatal(
                0x53B,
                "§2 r1",
                format!("LevelType {}: range first {first}, end −1", k.level_type),
            ));
        }
        // r2: first record in [first, end) that matches.
        let (lo, hi) = (first.max(0) as usize, end.max(0) as usize);
        let rows = self.records.get(lo..hi).ok_or_else(|| {
            AutomapError::Unresolved(format!(
                "§2 r2: range ({first}, {end}) past the {} records",
                self.records.len()
            ))
        })?;
        let hit = rows.iter().find(|r| {
            i64::from(r.level) == i64::from(k.level_type)
                && i64::from(r.tile) == i64::from(k.orientation)
                && (r.style == 0xFF || u32::from(r.style) == k.main)
                && (r.start == 0xFF || (u32::from(r.start) <= k.sub && k.sub <= u32::from(r.end)))
        });
        // r3: k = roll(count), Cel[k + 1] (1-based: `cels[k]`).
        let Some(r) = hit else { return Ok(-1) };
        let i = seed.roll(r.count) as usize;
        r.cels.get(i).copied().ok_or_else(|| {
            AutomapError::Unresolved(format!("§2 r3: roll {i} past Cel4 (count {})", r.count))
        })
    }
}

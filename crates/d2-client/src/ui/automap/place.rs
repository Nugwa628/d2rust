//! Tile records and units → cells.

use d2_sim::rng::Seed;

use super::cells::{Cell, CellTree};
use super::picker::{CelPicker, PickKey};
use super::AutomapError;
use crate::rules::camera::ClientPos;
use crate::rules::draw_order::{TileRecord, MONSTER, OBJECT};

/// Record flag +0x14 0x40000: the record was offered to the automap.
pub const REC_AUTOMAP: u32 = 0x4_0000;
/// Unit flags +0xC4: bit 28 drawn, bit 29 added to the automap.
pub const UNIT_DRAWN: u32 = 0x1000_0000;
pub const UNIT_AUTOMAP: u32 = 0x2000_0000;
/// Record types ≥ 16 are the lower walls `ld`, `rd`, `fd`, `fi`.
pub const LOWER_WALL_TYPE: u32 = 16;

fn fit(v: i32, code: u32, rule: &'static str, what: &str) -> Result<i16, AutomapError> {
    i16::try_from(v)
        .map_err(|_| AutomapError::fatal(code, rule, format!("{what} {v} is not an i16")))
}

/// R3–r4: the cell of world tile (wx, wy) of record type `ty`, cel `c`.
pub fn tile_cell(wx: i32, wy: i32, ty: u32, c: i32) -> Result<Cell, AutomapError> {
    let x = ((wx - wy) * 80) / 10;
    let mut y = ((wx + wy) * 40) / 10;
    if ty >= LOWER_WALL_TYPE {
        y += 24;
    }
    // Checked x, y, then the cel (fatal 0x391, 0x392, 0x393).
    let x = fit(x, 0x391, "§3 r4", "x")?;
    let y = fit(y, 0x392, "§3 r4", "y")?;
    let cel = fit(c, 0x393, "§3 r4", "cel")?;
    Ok(Cell::new(cel, x, y))
}

/// Offers record `rec` of a room with tile origin `origin` (room
/// +0x34/+0x38) and level type `level_type` to `tree`. `Ok(None)` when
/// the record was already offered or the picker gives −1; else whether
/// the insert took the cell.
pub fn add_tile(
    rec: &mut TileRecord,
    origin: (i32, i32),
    level_type: u32,
    picker: &CelPicker,
    seed: &mut Seed,
    tree: &mut CellTree,
) -> Result<Option<bool>, AutomapError> {
    // r1: once per record, whether or not a cell results (edge case 2).
    if rec.flags & REC_AUTOMAP != 0 {
        return Ok(None);
    }
    rec.flags |= REC_AUTOMAP;
    // r2.
    let c = picker.pick(
        PickKey {
            level_type,
            orientation: rec.dt1.orientation,
            main: rec.dt1.main,
            sub: rec.dt1.sub,
        },
        seed,
    )?;
    if c == -1 {
        return Ok(None);
    }
    // r3–r4.
    let (wx, wy) = (origin.0 + rec.tile.0, origin.1 + rec.tile.1);
    let cell = tile_cell(wx, wy, rec.ty, c)?;
    Ok(Some(tree.insert(cell)))
}

/// The cell of a unit at client position `p` with cel `c`.
pub fn unit_cell(p: ClientPos, c: i32) -> Result<Cell, AutomapError> {
    let x = fit(p.x / 10 + 1, 0x3C5, "§4 r4", "x")?;
    let y = fit(p.y / 10 - 3, 0x3C6, "§4 r4", "y")?;
    let cel = fit(c, 0x3C7, "§4 r4", "cel")?;
    Ok(Cell::new(cel, x, y))
}

/// What reads of one unit of a room's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AutomapUnit {
    /// 0 player, 1 monster, 2 object, …
    pub unit_type: u8,
    /// Class id (monstats row, objects row).
    pub class: u32,
    /// Mode (+0x10).
    pub mode: u32,
    /// Flags +0xC4 (bit 29 written here).
    pub flags: u32,
    /// Client position.
    pub pos: ClientPos,
}

/// The table values reads.
pub trait UnitCels {
    /// `monstats2` `automapCel` of monster class `class` (through monstats
    /// +0x18); `None` when the chain does not resolve.
    fn monster_cel(&self, class: u32) -> Option<u32>;
    /// `objects` `AutoMap` (+0x1BC) of object class `class`.
    fn object_cel(&self, class: u32) -> Option<u32>;
}

/// The level facts reads: its id and act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UnitLevel {
    pub id: u32,
    pub act: u8,
}

/// The add-all flag: nothing sets it, so false.
pub const ADD_ALL: bool = false;

/// R2–r3: the cel a unit adds, `None` for none.
pub fn unit_cel(u: &AutomapUnit, level: UnitLevel, cels: &dyn UnitCels) -> Option<u32> {
    match u.unit_type {
        MONSTER => cels.monster_cel(u.class).filter(|&c| c != 0),
        OBJECT => {
            let c = cels.object_cel(u.class).filter(|&c| c != 0)?;
            let ok = match u.class {
                267 => matches!(level.act, 2 | 3),
                366 => u.mode == 2,
                402 => level.id == 74,
                _ => true,
            };
            ok.then_some(c)
        }
        _ => None,
    }
}

/// Walks a room's units in list order into `tree`.
pub fn add_units(
    units: &mut [AutomapUnit],
    level: UnitLevel,
    cels: &dyn UnitCels,
    tree: &mut CellTree,
) -> Result<(), AutomapError> {
    for u in units {
        // r1.
        if !ADD_ALL && (u.flags & UNIT_DRAWN == 0 || u.flags & UNIT_AUTOMAP != 0) {
            continue;
        }
        u.flags |= UNIT_AUTOMAP;
        // r2–r4.
        if let Some(c) = unit_cel(u, level, cels) {
            let c = i32::try_from(c).unwrap_or(i32::MAX);
            tree.insert(unit_cell(u.pos, c)?);
        }
    }
    Ok(())
}

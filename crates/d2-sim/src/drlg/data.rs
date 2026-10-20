//! The table view the DRLG reads, built from `d2_data::tables` records (`leveldefs`,
//! `lvlwarp`, `lvltypes`, `objects`). Only the columns the level generator uses are kept.
//!
//! It also holds two code-side tables: [`WallRemap`], embedded from
//! `tables/drlg/wall-remap.tsv`, and [`DoorTables`], which has no built-in values and is
//! an input supplied by the caller.

use d2_data::tables::{text, Leveldefs, Lvltypes, Lvlwarp, Objects};

use super::DrlgError;

/// One `leveldefs` row (row index = level id), reduced to the columns the DRLG uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelDef {
    /// `DrlgType`: 1 maze, 2 preset, 3 outdoor; others generate nothing.
    pub drlg_type: u32,
    /// `LevelType`: `lvltypes` row.
    pub level_type: u32,
    /// `SizeX`, `SizeY` per difficulty (Normal, Nightmare, Hell).
    pub size: [(i32, i32); 3],
    /// `OffsetX`, `OffsetY`.
    pub offset: (i32, i32),
    /// `Depend`: level whose position is added (0 = none).
    pub depend: u32,
    /// `Vis0..7`: neighbouring level ids, 0 = none.
    pub vis: [u32; 8],
    /// `Warp0..7`: lvlwarp `Id`, −1 = none.
    pub warp: [i32; 8],
    /// `Position`.
    pub position: u32,
    /// `Portal`: ≠ 0 puts the level in the portal level list ([`DrlgData::portal_levels`]).
    pub portal: u32,
}

impl LevelDef {
    /// Builds the reduced row from a decoded `leveldefs` record.
    pub fn from_record(r: &Leveldefs) -> Self {
        let i = |v: u32| v as i32;
        Self {
            drlg_type: r.drlgtype,
            level_type: r.leveltype,
            size: [
                (i(r.sizex), i(r.sizey)),
                (i(r.sizex_n), i(r.sizey_n)),
                (i(r.sizex_h), i(r.sizey_h)),
            ],
            offset: (i(r.offsetx), i(r.offsety)),
            depend: r.depend,
            vis: [
                r.vis0, r.vis1, r.vis2, r.vis3, r.vis4, r.vis5, r.vis6, r.vis7,
            ],
            warp: [
                i(r.warp0),
                i(r.warp1),
                i(r.warp2),
                i(r.warp3),
                i(r.warp4),
                i(r.warp5),
                i(r.warp6),
                i(r.warp7),
            ],
            position: r.position,
            portal: r.portal,
        }
    }
}

/// One `lvlwarp` row (file order kept).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarpDef {
    /// `Id`.
    pub id: i32,
    /// `Direction`: `b'b'`, `b'l'`, `b'r'`, ...
    pub direction: u8,
    /// `LitVersion`.
    pub lit_version: u32,
    /// `Tiles`.
    pub tiles: u32,
    /// `OffsetX` (signed): the warp tile unit's x offset from 5 × its cell.
    pub offset_x: i32,
    /// `OffsetY` (signed): the warp tile unit's y offset from 5 × its cell.
    pub offset_y: i32,
    /// `SelectX`, `SelectY`, `SelectDX`, `SelectDY`: the warp tile's hover rectangle,
    /// relative to the tile's client pixel point.
    pub select: (i32, i32, i32, i32),
}

impl WarpDef {
    /// Builds the reduced row from a decoded `lvlwarp` record.
    pub fn from_record(r: &Lvlwarp) -> Self {
        Self {
            id: r.id as i32,
            direction: r.direction[0],
            lit_version: r.litversion,
            tiles: r.tiles,
            offset_x: r.offsetx as i32,
            offset_y: r.offsety as i32,
            select: (
                r.selectx as i32,
                r.selecty as i32,
                r.selectdx as i32,
                r.selectdy as i32,
            ),
        }
    }
}

/// Contents of `tables/drlg/wall-remap.tsv`, the wall-type remap table.
pub const WALL_REMAP_TSV: &str = include_str!("../../../../tables/drlg/wall-remap.tsv");

/// Number of new cell types the remap table covers.
pub const WALL_REMAP_TYPES: usize = 20;

/// The class of a new cell type (column `class` of the remap table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallClass {
    /// Merged type = entry `R.type` (columns `r0`..`r7`) when R's type is
    /// ≤ 7, else stop.
    Table([u32; 8]),
    /// Merged type = the new type, whatever R's type.
    Keep,
    /// No merge: the record is left as it is, no flag rules.
    Stop,
}

/// Wall-type remap of linked cells: an index of one class per new type 0..19, backed
/// by a 6×7 table of merged types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallRemap {
    /// Row = new cell type.
    pub classes: [WallClass; WALL_REMAP_TYPES],
}

impl Default for WallRemap {
    /// The built-in table ([`WallRemap::original`]).
    fn default() -> Self {
        Self::original()
    }
}

impl WallRemap {
    /// The built-in table, parsed from [`WALL_REMAP_TSV`].
    pub fn original() -> Self {
        Self::parse(WALL_REMAP_TSV).expect("tables/drlg/wall-remap.tsv is well-formed")
    }

    /// Strict parse of the `wall-remap.tsv` layout: the header, then 20 rows of 10
    /// columns, `new_type` equal to the row index; `table` rows carry 8 values, `keep`
    /// and `stop` rows 8 empty cells.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let header = "new_type\tclass\tr0\tr1\tr2\tr3\tr4\tr5\tr6\tr7";
        if lines.next() != Some(header) {
            return Err("bad header".into());
        }
        let mut classes = [WallClass::Stop; WALL_REMAP_TYPES];
        let mut n = 0;
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 10 {
                return Err(format!("row {n}: {} columns", f.len()));
            }
            if n >= WALL_REMAP_TYPES || f[0] != n.to_string() {
                return Err(format!("row {n}: new_type {:?}", f[0]));
            }
            let empty = f[2..].iter().all(|c| c.is_empty());
            classes[n] = match f[1] {
                "keep" if empty => WallClass::Keep,
                "stop" if empty => WallClass::Stop,
                "table" => {
                    let mut r = [0u32; 8];
                    for (v, c) in r.iter_mut().zip(&f[2..]) {
                        *v = c.parse().map_err(|_| format!("row {n}: value {c:?}"))?;
                    }
                    WallClass::Table(r)
                }
                c => return Err(format!("row {n}: class {c:?}")),
            };
            n += 1;
        }
        if n != WALL_REMAP_TYPES {
            return Err(format!("{n} rows"));
        }
        Ok(Self { classes })
    }

    /// The built-in classes with `table` as the 6×7 table (rows for new types 1, 2, 3,
    /// 5, 6, 7; column = R's type − 1). Entry `r0` of each row takes the last value of
    /// the previous row (0 for the first).
    #[cfg(test)]
    pub(crate) fn with_rows(table: [[u32; 7]; 6]) -> Self {
        let mut m = Self::original();
        let mut before = 0;
        for (i, t) in [1, 2, 3, 5, 6, 7].into_iter().enumerate() {
            let mut r = [before; 8];
            r[1..].copy_from_slice(&table[i]);
            before = table[i][6];
            m.classes[t] = WallClass::Table(r);
        }
        m
    }

    /// The class of new cell type `t`; `None` beyond the 20 rows.
    pub fn class(&self, t: u32) -> Option<WallClass> {
        self.classes.get(t as usize).copied()
    }
}

/// Door unit tables, for the door hook ([`super::LevelTypes::door_unit`]). There are no
/// built-in values; the tables are empty by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoorTables {
    /// (level id, first row, last row).
    pub levels: Vec<(u32, u32, u32)>,
    /// main, sub, right-door flag, unit id, unit type, dx, dy.
    pub rows: Vec<[i32; 7]>,
}

/// Everything the DRLG reads from tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrlgData {
    /// `leveldefs`, row = level id.
    pub levels: Vec<LevelDef>,
    /// `lvlwarp` rows in file order.
    pub warps: Vec<WarpDef>,
    /// `ExitWalkX`, `ExitWalkY` per row of [`Self::warps`] (a missing entry reads as
    /// (0, 0)): the walk-out on arriving through a warp.
    pub warp_exits: Vec<(i32, i32)>,
    /// `lvltypes` `File 1..32` per row (level type), NUL-trimmed.
    pub lvltypes: Vec<Vec<Vec<u8>>>,
    /// `objects` subclass per object class (row).
    pub object_subclass: Vec<u8>,
    /// See [`WallRemap`] (the built-in table by default).
    pub wall_remap: WallRemap,
    /// See [`DoorTables`].
    pub doors: DoorTables,
}

impl DrlgData {
    /// The view from decoded records (`d2_data::tables::decode_all`).
    pub fn from_tables(
        leveldefs: &[Leveldefs],
        lvlwarp: &[Lvlwarp],
        lvltypes: &[Lvltypes],
        objects: &[Objects],
    ) -> Self {
        Self {
            levels: leveldefs.iter().map(LevelDef::from_record).collect(),
            warps: lvlwarp.iter().map(WarpDef::from_record).collect(),
            warp_exits: lvlwarp
                .iter()
                .map(|r| (r.exitwalkx as i32, r.exitwalky as i32))
                .collect(),
            lvltypes: lvltypes.iter().map(lvltype_files).collect(),
            object_subclass: objects.iter().map(|o| o.subclass).collect(),
            wall_remap: WallRemap::original(),
            doors: DoorTables::default(),
        }
    }

    /// The portal level list: the level ids whose `Portal` ≠ 0, in order.
    pub fn portal_levels(&self) -> Vec<u32> {
        (0u32..)
            .zip(&self.levels)
            .filter(|(_, l)| l.portal != 0)
            .map(|(i, _)| i)
            .collect()
    }

    /// The leveldefs row of a level id.
    pub fn level(&self, id: u32) -> Result<&LevelDef, DrlgError> {
        self.levels
            .get(id as usize)
            .ok_or(DrlgError::UnknownLevel(id))
    }

    /// The first lvlwarp row (file order) with `Id` = `id` whose direction matches `dir`
    /// (`'b'` requests or rows match anything). Returns the row index.
    pub fn lvlwarp_row(&self, id: i32, dir: u8) -> Result<usize, DrlgError> {
        self.warps
            .iter()
            .position(|w| w.id == id && (dir == b'b' || w.direction == b'b' || w.direction == dir))
            .ok_or(DrlgError::NoLvlWarp(id))
    }

    /// `lvltypes` `File(i+1)` of a level type, if non-empty.
    pub fn lvltype_file(&self, level_type: u32, i: usize) -> Option<&[u8]> {
        let f = self.lvltypes.get(level_type as usize)?.get(i)?;
        (!f.is_empty()).then_some(f.as_slice())
    }

    /// Whether an object class has the waypoint bit (0x40) in its `objects.txt` subclass.
    pub fn is_waypoint_object(&self, class: u32) -> bool {
        self.object_subclass
            .get(class as usize)
            .is_some_and(|s| s & 0x40 != 0)
    }
}

fn lvltype_files(r: &Lvltypes) -> Vec<Vec<u8>> {
    [
        &r.file_1, &r.file_2, &r.file_3, &r.file_4, &r.file_5, &r.file_6, &r.file_7, &r.file_8,
        &r.file_9, &r.file_10, &r.file_11, &r.file_12, &r.file_13, &r.file_14, &r.file_15,
        &r.file_16, &r.file_17, &r.file_18, &r.file_19, &r.file_20, &r.file_21, &r.file_22,
        &r.file_23, &r.file_24, &r.file_25, &r.file_26, &r.file_27, &r.file_28, &r.file_29,
        &r.file_30, &r.file_31, &r.file_32,
    ]
    .iter()
    .map(|f| text(&f[..]).to_vec())
    .collect()
}

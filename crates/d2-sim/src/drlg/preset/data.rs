//! The table view preset code reads: lvlprest rows, monpreset rows
//! and act ranges, counts, and the two code tables of
//! `tables/drlg/preset-tables.tsv` (object-preset and door tables),
//! embedded and parsed strictly.

use d2_data::fixup::maps::ActRanges;
use d2_data::tables::{text, Lvlprest};

use super::PresetError;

/// `tables/drlg/preset-tables.tsv`.
pub const PRESET_TABLES_TSV: &str = include_str!("../../../../../tables/drlg/preset-tables.tsv");

/// Object-preset table width: DS1 object ids 0..149 per act.
pub const OBJPRESET_IDS: usize = 150;
/// DS1 acts 0..4.
pub const ACTS: usize = 5;

/// One lvlprest row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresetDef {
    pub def: u32,
    pub level_id: u32,
    pub populate: u32,
    pub logicals: u32,
    pub outdoors: u32,
    pub animate: u32,
    pub kill_edge: u32,
    pub fill_blanks: u32,
    pub size_x: u32,
    pub size_y: u32,
    pub automap: u32,
    pub scan: u32,
    pub pops: u32,
    /// `PopPad` (signed; −4 in 130 rows).
    pub pop_pad: i32,
    /// `Files` (read as the signed int `roll` takes).
    pub files: i32,
    /// `File1`–`File6`, NUL-trimmed.
    pub file: [Vec<u8>; 6],
    pub dt1_mask: u32,
    /// lvlprest +0x24 (animation speed argument): no column fills it,
    /// so 0 in every row of the base game.
    pub anim_speed: u32,
}

impl PresetDef {
    pub fn from_record(r: &Lvlprest) -> Self {
        let f = |b: &[u8]| text(b).to_vec();
        Self {
            def: r.def,
            level_id: r.levelid,
            populate: r.populate,
            logicals: r.logicals,
            outdoors: r.outdoors,
            animate: r.animate,
            kill_edge: r.killedge,
            fill_blanks: r.fillblanks,
            size_x: r.sizex,
            size_y: r.sizey,
            automap: r.automap,
            scan: r.scan,
            pops: r.pops,
            pop_pad: r.poppad as i32,
            files: r.files as i32,
            file: [
                f(&r.file1),
                f(&r.file2),
                f(&r.file3),
                f(&r.file4),
                f(&r.file5),
                f(&r.file6),
            ],
            dt1_mask: r.dt1mask,
            anim_speed: 0,
        }
    }
}

/// One monpreset row as `cb(monpreset.place)` stores it
/// (the monpreset table's place callback): kind u8 at +1, index u16 at +2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonPresetRow {
    /// 0 monplace, 1 monstats, 2 superuniques.
    pub kind: u8,
    pub place: u16,
}

impl MonPresetRow {
    /// From the 4-byte `monpreset.bin` record (the generated
    /// `d2_data::tables::Monpreset` decodes only `Act`).
    pub fn from_record_bytes(r: &[u8; 4]) -> Self {
        Self {
            kind: r[1],
            place: u16::from_le_bytes([r[2], r[3]]),
        }
    }
}

/// One door-table row (`preset-tables.tsv` table `door`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoorRow {
    pub main: u32,
    pub sub: u32,
    pub right: bool,
    pub unit_type: u32,
    pub class: i32,
    pub dx: i32,
    pub dy: i32,
}

/// The two built-in code tables (`preset-tables.tsv`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetTables {
    /// Class id per (act, DS1 object id); absent = 0.
    pub objpreset: [[i32; OBJPRESET_IDS]; ACTS],
    /// Door table: level id and its rows in table order, entries in table
    /// order.
    pub doors: Vec<(u32, Vec<DoorRow>)>,
}

const HEADER: &str = "table\ta\tb\tc\td\te\tf\tg\th";

impl PresetTables {
    /// The embedded table.
    pub fn spec() -> Result<Self, PresetError> {
        Self::from_tsv(PRESET_TABLES_TSV)
    }

    /// Strict parse: exact header, 9 columns, integer cells, objpreset
    /// keys in range and unique, objpreset cells e–h empty, door rows of
    /// one level contiguous. Anything else is an error.
    pub fn from_tsv(tsv: &str) -> Result<Self, PresetError> {
        let bad = |line: usize, why: &str| PresetError::Tsv(format!("line {line}: {why}"));
        let mut lines = tsv.lines();
        if lines.next() != Some(HEADER) {
            return Err(bad(1, "header"));
        }
        let mut objpreset = [[0i32; OBJPRESET_IDS]; ACTS];
        let mut seen = [[false; OBJPRESET_IDS]; ACTS];
        let mut doors: Vec<(u32, Vec<DoorRow>)> = Vec::new();
        for (i, l) in lines.enumerate() {
            let n = i + 2;
            let cols: Vec<&str> = l.split('\t').collect();
            if cols.len() != 9 {
                return Err(bad(n, "column count"));
            }
            let int = |c: &str| c.parse::<i32>().map_err(|_| bad(n, "integer"));
            match cols[0] {
                "objpreset" => {
                    if cols[4..].iter().any(|c| !c.is_empty()) {
                        return Err(bad(n, "objpreset cells d–h must be empty"));
                    }
                    let (a, b, c) = (int(cols[1])?, int(cols[2])?, int(cols[3])?);
                    if !(0..ACTS as i32).contains(&a) || !(0..OBJPRESET_IDS as i32).contains(&b) {
                        return Err(bad(n, "objpreset key out of range"));
                    }
                    let (a, b) = (a as usize, b as usize);
                    if seen[a][b] {
                        return Err(bad(n, "duplicate objpreset key"));
                    }
                    seen[a][b] = true;
                    objpreset[a][b] = c;
                }
                "door" => {
                    let v: Vec<i32> = cols[1..].iter().map(|c| int(c)).collect::<Result<_, _>>()?;
                    if v[0] < 0 || v[1] < 0 || v[2] < 0 || !(0..=1).contains(&v[3]) || v[4] < 0 {
                        return Err(bad(n, "door value out of range"));
                    }
                    let row = DoorRow {
                        main: v[1] as u32,
                        sub: v[2] as u32,
                        right: v[3] == 1,
                        unit_type: v[4] as u32,
                        class: v[5],
                        dx: v[6],
                        dy: v[7],
                    };
                    let level = v[0] as u32;
                    match doors.last_mut() {
                        Some((l, rows)) if *l == level => rows.push(row),
                        _ => {
                            if doors.iter().any(|(l, _)| *l == level) {
                                return Err(bad(n, "door rows of a level not contiguous"));
                            }
                            doors.push((level, vec![row]));
                        }
                    }
                }
                _ => return Err(bad(n, "unknown table")),
            }
        }
        Ok(Self { objpreset, doors })
    }

    /// The act's count of entries before its first 0 (150
    /// for a full act; D2MOO returns 0 there). Acts outside 0..4: 0.
    pub fn objpreset_count(&self, act: i32) -> u32 {
        match usize::try_from(act)
            .ok()
            .and_then(|a| self.objpreset.get(a))
        {
            Some(row) => row.iter().take_while(|&&c| c != 0).count() as u32,
            None => 0,
        }
    }

    /// Door lookup: the level's entry, then its first
    /// row with main, sub and right flag equal.
    pub fn door(&self, level_id: u32, main: u32, sub: u32, right: bool) -> Option<&DoorRow> {
        let (_, rows) = self.doors.iter().find(|(l, _)| *l == level_id)?;
        rows.iter()
            .find(|r| r.main == main && r.sub == sub && r.right == right)
    }
}

/// Everything preset code reads from tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetData {
    /// lvlprest rows in table order (row = `Def`).
    pub defs: Vec<PresetDef>,
    /// monpreset act ranges.
    pub monpreset_acts: ActRanges,
    /// monpreset rows in table order.
    pub monpreset: Vec<MonPresetRow>,
    /// monstats row count M (734 in the base game).
    pub monstats_count: u32,
    /// superuniques row count S (66 in the base game).
    pub superuniques_count: u32,
    /// Item class of the code `hdm ` (lookup in the one-entry
    /// item code table), −1 if the item table has none.
    pub hdm_item: i32,
    pub tables: PresetTables,
}

impl PresetData {
    /// First row (table order) with `LevelId` = `id`.
    pub fn def_for_level(&self, id: u32) -> Option<u32> {
        self.defs
            .iter()
            .position(|d| d.level_id == id)
            .map(|i| i as u32)
    }

    /// Row `i` if in range.
    pub fn def(&self, i: u32) -> Result<&PresetDef, PresetError> {
        self.defs.get(i as usize).ok_or(PresetError::UnknownDef(i))
    }
}

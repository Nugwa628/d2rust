//! Loads the game's compiled `.bin` tables from the player's own archives.
//!
//! This module covers the excel path rule, resolving each file through the
//! archive set (or another [`TableFiles`] source), the `.bin` container and its
//! strict size check, the table load sequence, the check that server-only
//! files are absent, the raw formula code buffers, the runtime `.txt` sound
//! tables, and the per-table count and content checks run after each table
//! loads. The records keep their shipped bytes; fix-ups are applied to a copy
//! by [`crate::fixup`]. The result, a [`BinSet`], is what `compile_set`,
//! `crosscheck` and `fixup` build on.

use std::collections::BTreeMap;

use d2_formats::mpq::{ArchiveSet, MpqError};

use crate::calc::{self, BufferError, BufferReport, Family};
use crate::compile::CodeLinker;
use crate::schema::{schema, CalcBuffer, FieldType, Link, TableDef};
use crate::strings::{StringTables, StringsError};
use crate::txt::{TxtError, TxtTable};
use d2_formats::tbl::StringTable;

/// Archive directory that holds the excel tables (`DATA\GLOBAL\EXCEL\`).
pub const EXCEL_DIR: &str = "data\\global\\excel\\";
/// The language whose string tables are loaded.
pub const DEFAULT_LANGUAGE: &str = "eng";

/// The archive path of an excel file (`armor.bin` → `data\global\excel\armor.bin`).
pub fn excel_path(file: &str) -> String {
    format!("{EXCEL_DIR}{file}")
}

/// A load failure. Loading is all-or-nothing: the first error stops it.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The archive set failed to read `file`.
    #[error("{file}: {source}")]
    Mpq { file: String, source: MpqError },
    /// No archive holds `file`.
    #[error("{file}: not found in any archive")]
    Missing { file: String },
    /// A read error from a non-archive [`TableFiles`] source (native).
    #[error("{file}: {detail}")]
    Source { file: String, detail: String },
    /// The file is shorter than its 4-byte record count.
    #[error("{file}: {len} bytes, shorter than the 4-byte count")]
    TooShort { file: String, len: usize },
    /// The file length is not `4 + count * record_size`.
    #[error("{file}: {len} bytes, expected 4 + {count} x {record_size} = {expected}")]
    SizeMismatch {
        file: String,
        len: usize,
        count: u32,
        record_size: usize,
        expected: u64,
    },
    /// A server-only file is present where it must be absent.
    #[error("{file}: server-only file present (checked before `{table}`)")]
    ServerFile { file: String, table: String },
    /// A post-load count or content check on `table` failed.
    #[error("{table}: {rule}")]
    Check { table: String, rule: String },
    /// A formula code buffer failed validation.
    #[error("{buffer}.bin: {detail}")]
    CodeBuffer {
        buffer: &'static str,
        detail: String,
    },
    /// A string table failed to load.
    #[error(transparent)]
    Strings(#[from] StringsError),
    /// A runtime `.txt` table failed to parse.
    #[error(transparent)]
    Txt(#[from] TxtError),
}

fn check(table: &str, rule: impl Into<String>) -> LoadError {
    LoadError::Check {
        table: table.to_owned(),
        rule: rule.into(),
    }
}

/// Where the table loader reads its files: the archive set (`Mpq`, today)
/// or a converted native folder (implemented in `d2-native::source`). Both give the loader the same bytes: the native
/// source serves each `.bin` as the compile of its native `.txt` with the
/// overrides and mod patches applied.
pub trait TableFiles {
    /// `(origin, bytes)` of the excel file `file` (`armor.bin`), or `None`
    /// when the source has none.
    fn read_excel(&self, file: &str) -> Result<Option<(String, Vec<u8>)>, LoadError>;
    /// `d2exp.mpq` is present (or the native conversion is of LoD).
    fn lod(&self) -> bool;
    /// The string table at `path` (`data\local\lng\eng\string.tbl`).
    fn string_table(&self, path: &str) -> Result<StringTable, StringsError>;
    /// The mod string edits (`stradd` / `strset`) to apply over the string
    /// tables after they load, in stack order; none by default.
    fn string_edits(&self) -> Vec<crate::strings::StringEdit> {
        Vec::new()
    }
}

impl TableFiles for ArchiveSet {
    fn read_excel(&self, file: &str) -> Result<Option<(String, Vec<u8>)>, LoadError> {
        read_excel(self, file)
    }

    fn lod(&self) -> bool {
        self.has_archive("d2exp.mpq")
    }

    fn string_table(&self, path: &str) -> Result<StringTable, StringsError> {
        let bytes = self.read(path).map_err(|source| StringsError::Mpq {
            file: path.to_owned(),
            source,
        })?;
        StringTable::parse(&bytes).map_err(|source| StringsError::Format {
            file: path.to_owned(),
            source,
        })
    }
}

/// Reads an excel file through the archive set: `(archive, bytes)`, or
/// `None` when no archive has it.
pub fn read_excel(set: &ArchiveSet, file: &str) -> Result<Option<(String, Vec<u8>)>, LoadError> {
    let path = excel_path(file);
    set.read_with_source(&path)
        .map_err(|source| LoadError::Mpq { file: path, source })
}

/// A `.bin` record table: a 4-byte count followed by fixed-size records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinTable {
    /// Table name as in the schema (`armor`).
    pub name: String,
    /// Archive the file was read from (lowercase file name).
    pub source: String,
    /// Number of records.
    pub count: usize,
    /// Bytes per record.
    pub record_size: usize,
    /// `count × record_size` bytes.
    pub records: Vec<u8>,
}

impl BinTable {
    /// Splits a `.bin` and checks its size strictly: at least 4 bytes, and the length
    /// must equal `4 + count * record_size`.
    pub fn parse(
        name: &str,
        source: &str,
        file: &str,
        data: &[u8],
        record_size: usize,
    ) -> Result<BinTable, LoadError> {
        if data.len() < 4 {
            return Err(LoadError::TooShort {
                file: file.to_owned(),
                len: data.len(),
            });
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        // Saturating: no .bin is near u64::MAX bytes, so a saturated size
        // never equals the length and stays a size mismatch.
        let expected = u64::from(count)
            .saturating_mul(record_size as u64)
            .saturating_add(4);
        if data.len() as u64 != expected {
            return Err(LoadError::SizeMismatch {
                file: file.to_owned(),
                len: data.len(),
                count,
                record_size,
                expected,
            });
        }
        Ok(BinTable {
            name: name.to_owned(),
            source: source.to_owned(),
            count: count as usize,
            record_size,
            records: data[4..].to_vec(),
        })
    }

    /// The bytes of record `i`; panics if `i` is out of range.
    pub fn record(&self, i: usize) -> &[u8] {
        &self.records[i * self.record_size..(i + 1) * self.record_size]
    }

    /// All records in file order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.records.chunks_exact(self.record_size.max(1))
    }
}

/// Little-endian reads at record offsets.
pub fn u32_at(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        record[offset],
        record[offset + 1],
        record[offset + 2],
        record[offset + 3],
    ])
}

/// The NUL-terminated string at `range` of a record (without the NUL).
pub fn cstr(record: &[u8], range: std::ops::Range<usize>) -> &[u8] {
    let s = &record[range.start..range.end.min(record.len())];
    let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    &s[..n]
}

/// A raw formula code buffer, as shipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFile {
    /// Which buffer this is.
    pub buffer: CalcBuffer,
    /// Archive the file was read from.
    pub source: String,
    /// The file contents.
    pub bytes: Vec<u8>,
}

/// The tables whose formula fields point into a code buffer.
pub fn buffer_tables(buffer: CalcBuffer) -> &'static [&'static str] {
    match buffer {
        CalcBuffer::MissCode => &["missiles"],
        CalcBuffer::SkillsCode => &["skills"],
        CalcBuffer::SkillDescCode => &["skilldesc"],
        CalcBuffer::ItemsCode => &["weapons", "armor", "misc"],
    }
}

/// Code buffers loaded right after a table.
fn buffers_after(table: &str) -> &'static [CalcBuffer] {
    match table {
        "missiles" => &[CalcBuffer::MissCode],
        "skilldesc" => &[CalcBuffer::SkillsCode, CalcBuffer::SkillDescCode],
        "misc" => &[CalcBuffer::ItemsCode],
        _ => &[],
    }
}

/// Server-only files that must be absent before a table loads.
fn server_files(table: &str) -> &'static [&'static str] {
    match table {
        "runes" => &["runessrv.txt", "runessrv.bin", "runessrv.xls"],
        "cubemain" => &["cubeserver.bin", "cubeserver.txt"],
        _ => &[],
    }
}

/// The check made before `table` loads: any of its server-only files
/// present in the archive set is fatal.
fn check_server_files(set: &dyn TableFiles, table: &str) -> Result<(), LoadError> {
    for file in server_files(table) {
        if set.read_excel(file)?.is_some() {
            return Err(LoadError::ServerFile {
                file: excel_path(file),
                table: table.to_owned(),
            });
        }
    }
    Ok(())
}

/// How D2RUST's data checks apply to a game's tables
/// (`docs/plans/mods/seams.md` MS10).
///
/// D2RUST's tables are checked against fixed row-count limits. A mod
/// game's tables may have more rows: under [`DataRules::Mod`] a table holds
/// as many rows as its data has, up to the width of the ids that name its
/// rows elsewhere in the records and in the simulation (MS10b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataRules {
    /// D2RUST's own checks.
    #[default]
    Original,
    /// A mod game's: row counts come from the loaded data.
    Mod,
}

impl DataRules {
    /// The most rows table `name` may have, `None` when it has no limit
    /// of this kind.
    ///
    /// Under both: `pettype` and `states` ids are bytes (255), and skill,
    /// unique, set and monster class ids are signed 16-bit (32,766).
    /// Under [`DataRules::Original`] `itemstatcost` is capped at 511 (the
    /// 9-bit stat id of the item bit layout) and `levels` at 1,023; under
    /// [`DataRules::Mod`] both reach 65,535, the width of the
    /// 16-bit ids that name them (0xFFFF stays free as "none"). Stats past
    /// 510 can't ride in D2RUST's item bit layout; a mod game that puts
    /// them on items needs its own layout (MS10c).
    pub fn max_count(self, name: &str) -> Option<usize> {
        let from_data = self == DataRules::Mod;
        match name {
            "pettype" | "states" => Some(255),
            "itemstatcost" if from_data => Some(65_535),
            "itemstatcost" => Some(511),
            "skills" | "skilldesc" | "uniqueitems" | "sets" | "setitems" | "monstats"
            | "monequip" => Some(32_766),
            "levels" if from_data => Some(65_535),
            "levels" => Some(1_023),
            _ => None,
        }
    }
}

/// Everything the game reads from its data tables at startup, validated.
#[derive(Debug, Clone)]
pub struct BinSet {
    /// `d2exp.mpq` is present.
    pub lod: bool,
    /// The data rules the set was checked under.
    pub rules: DataRules,
    /// The string tables for the loaded language.
    pub strings: StringTables,
    /// The 73 runtime record tables, in load order.
    pub tables: Vec<BinTable>,
    /// The raw formula code buffers.
    pub code: BTreeMap<CalcBuffer, CodeFile>,
    /// Validation of each code buffer against its formula fields.
    pub code_reports: BTreeMap<CalcBuffer, BufferReport>,
    /// `hitclass.bin`, read by the client composite loader.
    pub hitclass: BinTable,
    /// The runtime `sounds.txt` table.
    pub sounds: TxtTable,
    /// The runtime `soundenviron.txt` table.
    pub soundenviron: TxtTable,
}

impl BinSet {
    /// The loaded table called `name`, if any.
    pub fn table(&self, name: &str) -> Option<&BinTable> {
        self.tables.iter().find(|t| t.name == name)
    }
}

fn load_bin(set: &dyn TableFiles, def: &TableDef) -> Result<BinTable, LoadError> {
    let (source, bytes) = set
        .read_excel(&def.bin_name)?
        .ok_or_else(|| LoadError::Missing {
            file: excel_path(&def.bin_name),
        })?;
    BinTable::parse(
        &def.name,
        &source,
        &excel_path(&def.bin_name),
        &bytes,
        def.record_size,
    )
}

/// Loads and validates the live `.bin` set from an archive set.
pub fn load(set: &ArchiveSet, language: &str) -> Result<BinSet, LoadError> {
    load_from(set, language)
}

/// [`load`] over any [`TableFiles`] source, under D2RUST's own rules.
pub fn load_from(set: &dyn TableFiles, language: &str) -> Result<BinSet, LoadError> {
    load_from_with(set, language, DataRules::Original)
}

/// [`load_from`] under the data `rules` of the game being built (a mod
/// game passes [`DataRules::Mod`]).
pub fn load_from_with(
    set: &dyn TableFiles,
    language: &str,
    rules: DataRules,
) -> Result<BinSet, LoadError> {
    let lod = set.lod();
    let strings = StringTables::load_from(set, language, lod)?;
    let mut tables: Vec<BinTable> = Vec::new();
    let mut code = BTreeMap::new();
    for def in schema().runtime() {
        check_server_files(set, &def.name)?;
        let table = load_bin(set, def)?;
        post_load_check_with(&table, &tables, &strings, lod, rules)?;
        tables.push(table);
        for &buffer in buffers_after(&def.name) {
            let file = format!("{}.bin", buffer.name());
            let (source, bytes) = set.read_excel(&file)?.ok_or_else(|| LoadError::Missing {
                file: excel_path(&file),
            })?;
            code.insert(
                buffer,
                CodeFile {
                    buffer,
                    source,
                    bytes,
                },
            );
        }
    }

    let mut code_reports = BTreeMap::new();
    for (&buffer, file) in &code {
        let report = calc::validate_buffer(
            Family::of(buffer),
            &file.bytes,
            formula_fields(buffer, &tables),
        )
        .map_err(|e| LoadError::CodeBuffer {
            buffer: buffer.name(),
            detail: match e {
                BufferError::BadOpcode { offset, opcode } => {
                    format!("opcode {opcode:#04x} at {offset}")
                }
                BufferError::Truncated { offset } => format!("expression cut at {offset}"),
                BufferError::BadField { value } => {
                    format!("formula field {value} is not an expression start")
                }
                BufferError::MissileRand { offset } => format!("missile rand at {offset}"),
            },
        })?;
        code_reports.insert(buffer, report);
    }

    let hitclass_def = schema().table("hitclass").expect("hitclass in tables.tsv");
    let hitclass = load_bin(set, hitclass_def)?;

    let read_txt = |file: &str| -> Result<TxtTable, LoadError> {
        let (_, bytes) = set.read_excel(file)?.ok_or_else(|| LoadError::Missing {
            file: excel_path(file),
        })?;
        Ok(TxtTable::parse(&excel_path(file), &bytes)?)
    };
    Ok(BinSet {
        lod,
        rules,
        sounds: read_txt("sounds.txt")?,
        soundenviron: read_txt("soundenviron.txt")?,
        strings,
        tables,
        code,
        code_reports,
        hitclass,
    })
}

/// The formula field values that point into `buffer`.
pub fn formula_fields(buffer: CalcBuffer, tables: &[BinTable]) -> Vec<u32> {
    let mut values = Vec::new();
    for name in buffer_tables(buffer) {
        let (Some(def), Some(table)) = (
            schema().table(name),
            tables.iter().find(|t| &t.name == name),
        ) else {
            continue;
        };
        let offsets: Vec<usize> = def
            .fields
            .iter()
            .filter(|f| f.field_type == FieldType::CalcToDword && f.link == Link::Calc(buffer))
            .map(|f| f.offset as usize)
            .collect();
        for rec in table.iter() {
            values.extend(offsets.iter().map(|&o| u32_at(rec, o)));
        }
    }
    values
}

/// The runtime item code map: `code` (+0x80) of the combined weapons, armor and
/// misc records, added in order with the code linker's `add`.
pub fn item_code_map(tables: &[BinTable]) -> CodeLinker {
    let mut map = CodeLinker::default();
    for name in ["weapons", "armor", "misc"] {
        if let Some(t) = tables.iter().find(|t| t.name == name) {
            for rec in t.iter() {
                map.add(u32_at(rec, 0x80));
            }
        }
    }
    map
}

/// Number of treasure classes built at load: the empty
/// TC, 32 automatic TCs per itemtypes record with byte 0x1D ≠ 0, then the
/// `treasureclassex` rows up to the first empty name.
pub fn tc_count(itemtypes: &BinTable, treasureclassex: &BinTable) -> usize {
    let auto = itemtypes.iter().filter(|r| r[0x1D] != 0).count();
    let rows = treasureclassex.iter().take_while(|r| r[0] != 0).count();
    1 + 32 * auto + rows
}

/// The `LevelName` values the `automap` table may use.
pub const AUTOMAP_LEVEL_NAMES: [&str; 36] = [
    "None",
    "1 Town",
    "1 Wilderness",
    "1 Cave",
    "1 Crypt",
    "1 Monestary",
    "1 Courtyard",
    "1 Barracks",
    "1 Jail",
    "1 Cathedral",
    "1 Catacombs",
    "1 Tristram",
    "2 Town",
    "2 Sewer",
    "2 Harem",
    "2 Basement",
    "2 Desert",
    "2 Tomb",
    "2 Lair",
    "2 Arcane",
    "3 Town",
    "3 Jungle",
    "3 Kurast",
    "3 Spider",
    "3 Dungeon",
    "3 Sewer",
    "4 Town",
    "4 Mesa",
    "4 Lava",
    "5 Town",
    "5 Siege",
    "5 Barricade",
    "5 Temple",
    "5 Ice",
    "5 Baal",
    "5 Lava",
];

/// The `TileName` values the `automap` table may use.
pub const AUTOMAP_TILE_NAMES: [&str; 20] = [
    "fl", "wl", "wr", "wtlr", "wtll", "wtr", "wbl", "wbr", "wld", "wrd", "wle", "wre", "co", "sh",
    "tr", "rf", "ld", "rd", "fd", "fi",
];

/// The post-load count and content checks for
/// `t`, given the tables loaded before it, under D2RUST's own rules.
pub fn post_load_check(
    t: &BinTable,
    earlier: &[BinTable],
    strings: &StringTables,
    lod: bool,
) -> Result<(), LoadError> {
    post_load_check_with(t, earlier, strings, lod, DataRules::Original)
}

/// [`post_load_check`] under the data `rules` of the game.
pub fn post_load_check_with(
    t: &BinTable,
    earlier: &[BinTable],
    strings: &StringTables,
    lod: bool,
    rules: DataRules,
) -> Result<(), LoadError> {
    let name = t.name.as_str();
    let get = |n: &str| {
        earlier
            .iter()
            .find(|e| e.name == n)
            .ok_or_else(|| check(name, format!("needs `{n}` loaded first")))
    };
    if let Some(limit) = rules.max_count(name) {
        if t.count > limit {
            return Err(check(name, format!("count {} > {limit}", t.count)));
        }
    }
    let exactly = |n: usize| {
        if t.count != n {
            Err(check(name, format!("count {} != {n}", t.count)))
        } else {
            Ok(())
        }
    };
    match name {
        "inventory" => exactly(32),
        "difficultylevels" => exactly(3),
        "arena" => exactly(1),
        "composit" => exactly(16),
        "armtype" => exactly(3),
        "experience" => exactly(101),
        "belts" => {
            if t.count / 2 != 7 {
                Err(check(name, format!("count {} / 2 != 7", t.count)))
            } else {
                Ok(())
            }
        }
        "leveldefs" => {
            let levels = get("levels")?.count;
            if t.count != levels {
                Err(check(
                    name,
                    format!("count {} != levels count {levels}", t.count),
                ))
            } else {
                Ok(())
            }
        }
        "chartemplate" => {
            let mut highest = 0u8;
            for (i, r) in t.iter().enumerate() {
                if r[0x1F] < highest {
                    return Err(check(
                        name,
                        format!("record {i}: Level {} below {highest}", r[0x1F]),
                    ));
                }
                highest = r[0x1F];
            }
            Ok(())
        }
        "gamble" => {
            let items = item_code_map(earlier);
            for (i, r) in t.iter().enumerate() {
                let code = u32_at(r, 0);
                if items.find(code).is_none() {
                    return Err(check(
                        name,
                        format!("record {i}: code {:?} is not an item", code.to_le_bytes()),
                    ));
                }
            }
            Ok(())
        }
        "treasureclassex" => {
            let n = tc_count(get("itemtypes")?, t);
            if n > 65_534 {
                Err(check(name, format!("{n} treasure classes > 65534")))
            } else {
                Ok(())
            }
        }
        "superuniques" => {
            let rows: Vec<u32> = t.iter().take(512).map(|r| u32_at(r, 0x08)).collect();
            match (0..66).find(|v| !rows.contains(v)) {
                Some(v) => Err(check(name, format!("hcIdx {v} occurs in no row"))),
                None => Ok(()),
            }
        }
        "hireling" if lod => {
            for (i, r) in t.iter().enumerate() {
                let id = u32_at(r, 0x04);
                let first = strings.id(cstr(r, 0xD3..t.record_size));
                let last = strings.id(cstr(r, 0xF3..t.record_size));
                if id > 255 || first == 0 || last <= first {
                    return Err(check(
                        name,
                        format!("record {i}: Id {id}, NameFirst id {first}, NameLast id {last}"),
                    ));
                }
            }
            Ok(())
        }
        "automap" => {
            for (i, r) in t.iter().enumerate() {
                let level = cstr(r, 0..16);
                let tile = cstr(r, 16..24);
                let known = |s: &[u8], list: &[&str]| {
                    s.is_empty() || list.iter().any(|n| n.as_bytes() == s)
                };
                if !known(level, &AUTOMAP_LEVEL_NAMES) || !known(tile, &AUTOMAP_TILE_NAMES) {
                    return Err(check(
                        name,
                        format!(
                            "record {i}: unknown LevelName `{}` or TileName `{}`",
                            String::from_utf8_lossy(level),
                            String::from_utf8_lossy(tile)
                        ),
                    ));
                }
                if u32_at(r, 0x1C) == u32::MAX {
                    return Err(check(name, format!("record {i}: Cel1 is -1")));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(count: u32, size: usize, extra: usize) -> Vec<u8> {
        let mut d = count.to_le_bytes().to_vec();
        d.resize(4 + count as usize * size + extra, 0);
        d
    }

    fn table(name: &str, count: u32, size: usize) -> BinTable {
        BinTable::parse(name, "p", name, &bin(count, size, 0), size).unwrap()
    }

    #[test]
    fn container_size_rules() {
        assert!(matches!(
            BinTable::parse("x", "p", "x", &[1, 0, 0], 4),
            Err(LoadError::TooShort { .. })
        ));
        let t = BinTable::parse("levels", "p", "levels.bin", &bin(137, 544, 0), 544).unwrap();
        assert_eq!((t.count, t.records.len()), (137, 137 * 544));
        assert_eq!(4 + 137 * 544, 74_532);
        // armor.bin from d2exp: 202 records of the 592-byte layout.
        let x = bin(202, 592, 0);
        assert_eq!(x.len(), 119_588);
        assert!(matches!(
            BinTable::parse("armor", "x", "armor.bin", &x, 424),
            Err(LoadError::SizeMismatch {
                expected: 85_652,
                ..
            })
        ));
        assert!(BinTable::parse("a", "p", "a", &bin(1, 4, 1), 4).is_err());
    }

    #[test]
    fn count_checks() {
        let s = StringTables::default();
        let ok = |t: &BinTable, e: &[BinTable]| post_load_check(t, e, &s, true).is_ok();
        assert!(ok(&table("inventory", 32, 240), &[]));
        assert!(!ok(&table("inventory", 31, 240), &[]));
        assert!(!ok(&table("inventory", 33, 240), &[]));
        assert!(ok(&table("belts", 14, 264), &[]));
        assert!(ok(&table("belts", 15, 264), &[]));
        assert!(!ok(&table("belts", 16, 264), &[]));
        assert!(!ok(&table("difficultylevels", 4, 88), &[]));
        assert!(!ok(&table("experience", 100, 32), &[]));
        assert!(ok(&table("pettype", 255, 224), &[]));
        assert!(!ok(&table("pettype", 256, 224), &[]));
        let levels = table("levels", 137, 544);
        assert!(ok(
            &table("leveldefs", 137, 156),
            std::slice::from_ref(&levels)
        ));
        assert!(!ok(&table("leveldefs", 136, 156), &[levels]));
    }

    #[test]
    fn mod_limits_come_from_the_data() {
        let s = StringTables::default();
        let ok = |t: &BinTable, e: &[BinTable], l: DataRules| {
            post_load_check_with(t, e, &s, true, l).is_ok()
        };
        let (orig, data) = (DataRules::Original, DataRules::Mod);
        // itemstatcost and levels: D2RUST's caps; a mod's reach 65,535.
        let stats = table("itemstatcost", 600, 4);
        assert!(!ok(&stats, &[], orig));
        assert!(ok(&stats, &[], data));
        assert!(!ok(&table("itemstatcost", 65_536, 1), &[], data));
        let levels = table("levels", 2_000, 4);
        assert!(!ok(&levels, &[], orig));
        assert!(ok(&levels, &[], data));
        // leveldefs still matches the loaded levels count.
        let lv = std::slice::from_ref(&levels);
        assert!(ok(&table("leveldefs", 2_000, 4), lv, data));
        assert!(!ok(&table("leveldefs", 1_999, 4), lv, data));
        // Byte and signed 16-bit ids keep their width under both.
        for l in [orig, data] {
            assert!(!ok(&table("states", 256, 4), &[], l));
            assert!(ok(&table("skills", 32_766, 1), &[], l));
            assert!(!ok(&table("skills", 32_767, 1), &[], l));
        }
        // Original is what post_load_check checks.
        assert_eq!(DataRules::default(), orig);
        assert!(post_load_check(&stats, &[], &s, true).is_err());
    }

    #[test]
    fn chartemplate_levels() {
        let s = StringTables::default();
        let mut t = table("chartemplate", 2, 240);
        t.records[0x1F] = 9;
        t.records[240 + 0x1F] = 3;
        assert!(post_load_check(&t, &[], &s, true).is_err());
        t.records[240 + 0x1F] = 9;
        assert!(post_load_check(&t, &[], &s, true).is_ok());
    }

    #[test]
    fn automap_names() {
        let s = StringTables::default();
        let mut t = table("automap", 1, 44);
        t.records[..6].copy_from_slice(b"1 Town");
        t.records[16..18].copy_from_slice(b"fl");
        assert!(post_load_check(&t, &[], &s, true).is_ok());
        t.records[..6].copy_from_slice(b"6 Town");
        assert!(post_load_check(&t, &[], &s, true).is_err());
        t.records[..6].copy_from_slice(b"1 Town");
        t.records[0x1C..0x20].copy_from_slice(&[0xFF; 4]);
        assert!(post_load_check(&t, &[], &s, true).is_err());
    }

    #[test]
    fn superunique_coverage() {
        let s = StringTables::default();
        let mut t = table("superuniques", 66, 52);
        for i in 0..66 {
            t.records[i * 52 + 8] = i as u8;
        }
        assert!(post_load_check(&t, &[], &s, true).is_ok());
        t.records[5 * 52 + 8] = 70;
        assert!(post_load_check(&t, &[], &s, true).is_err());
    }

    #[test]
    fn check_offsets_match_schema() {
        let s = schema();
        let f = |t: &str, c: &str| s.table(t).unwrap().field(c).unwrap().offset;
        assert_eq!(f("chartemplate", "level"), 0x1F);
        assert_eq!(f("superuniques", "hcIdx"), 0x08);
        assert_eq!(f("hireling", "id"), 0x04);
        assert_eq!(f("hireling", "namefirst"), 0xD3);
        assert_eq!(f("hireling", "namelast"), 0xF3);
        assert_eq!(f("automap", "LevelName"), 0);
        assert_eq!(f("automap", "TileName"), 16);
        assert_eq!(f("automap", "Cel1"), 0x1C);
        assert_eq!(f("itemtypes", "treasureclass"), 0x1D);
        assert_eq!(f("weapons", "code"), 0x80);
        assert_eq!(f("gamble", "code"), 0);
    }

    /// Encrypts `plain` so that `crypto::decrypt(_, key)` gives it back:
    /// word by word, the key stream word is what decrypting a zero word
    /// after the already-encrypted prefix yields.
    fn encrypt(plain: &[u8], key: u32) -> Vec<u8> {
        use d2_formats::mpq::crypto::decrypt;
        let mut cipher = vec![0u8; plain.len()];
        for w in (0..plain.len()).step_by(4) {
            let mut probe = cipher[..w + 4].to_vec();
            decrypt(&mut probe, key);
            for k in 0..4 {
                cipher[w + k] = probe[w + k] ^ plain[w + k];
            }
        }
        cipher
    }

    /// A minimal format-0 MPQ: stored (uncompressed, single-unit) files.
    fn mpq_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        use d2_formats::mpq::crypto::{hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
        use d2_formats::mpq::flags;
        let n = (files.len() * 2).next_power_of_two().max(4);
        let mut out = vec![0u8; 32];
        let mut blocks = Vec::new();
        let mut hashes = vec![[u32::MAX; 4]; n];
        for (i, (name, bytes)) in files.iter().enumerate() {
            let len = bytes.len() as u32;
            blocks.push([
                out.len() as u32,
                len,
                len,
                flags::EXISTS | flags::SINGLE_UNIT,
            ]);
            out.extend_from_slice(bytes);
            let mut at = hash(name.as_bytes(), HashType::TableOffset) as usize & (n - 1);
            while hashes[at][3] != u32::MAX {
                at = (at + 1) & (n - 1);
            }
            hashes[at] = [
                hash(name.as_bytes(), HashType::NameA),
                hash(name.as_bytes(), HashType::NameB),
                0,
                i as u32,
            ];
        }
        let words = |t: &[[u32; 4]]| -> Vec<u8> {
            t.iter().flatten().flat_map(|w| w.to_le_bytes()).collect()
        };
        let hash_pos = out.len() as u32;
        out.extend(encrypt(&words(&hashes), HASH_TABLE_KEY));
        let block_pos = out.len() as u32;
        out.extend(encrypt(&words(&blocks), BLOCK_TABLE_KEY));
        let mut h = Vec::with_capacity(32);
        h.extend_from_slice(b"MPQ\x1A");
        for v in [32, out.len() as u32] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        h.extend_from_slice(&0u16.to_le_bytes()); // format version
        h.extend_from_slice(&3u16.to_le_bytes()); // sector size shift
        for v in [hash_pos, block_pos, n as u32, files.len() as u32] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        out[..32].copy_from_slice(&h);
        out
    }

    /// An archive's files: (path, bytes).
    type Files<'a> = &'a [(&'a str, &'a [u8])];

    /// A temporary install: `archives` = (archive file name, files).
    struct Install(std::path::PathBuf);

    impl Install {
        fn new(tag: &str, archives: &[(&str, Files)]) -> Install {
            let dir =
                std::env::temp_dir().join(format!("d2-data-bin-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, files) in archives {
                std::fs::write(dir.join(name), mpq_bytes(files)).unwrap();
            }
            Install(dir)
        }

        fn set(&self) -> ArchiveSet {
            ArchiveSet::open_dir(&self.0).unwrap()
        }
    }

    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An empty `.tbl` (21-byte header, no elements, no slots).
    fn empty_tbl() -> Vec<u8> {
        let mut t = vec![0u8; 21];
        t[8] = 0; // version
        t[9..13].copy_from_slice(&21u32.to_le_bytes()); // data start
        t[17..21].copy_from_slice(&21u32.to_le_bytes()); // file size
        t
    }

    #[test]
    fn excel_path_rule() {
        assert_eq!(excel_path("armor.bin"), "data\\global\\excel\\armor.bin");
        assert!(excel_path("armor.txt").eq_ignore_ascii_case(r"DATA\GLOBAL\EXCEL\armor.txt"));
        // Every called table's files are its name + `.bin` / `.txt`
        // (`_lookup` lists read the table they name; leveldefs reads
        // levels.txt).
        for def in schema().called() {
            let base = def.name.strip_suffix("_lookup").unwrap_or(&def.name);
            assert_eq!(def.bin_name, format!("{base}.bin"), "{}", def.name);
            let txt = if base == "leveldefs" { "levels" } else { base };
            assert_eq!(def.txt_name, format!("{txt}.txt"), "{}", def.name);
        }
    }

    #[test]
    fn table_names_lowercase_lookup_case_insensitive() {
        for def in schema().runtime() {
            assert_eq!(def.name, def.name.to_ascii_lowercase());
        }
        let txt: &[u8] = b"code\r\nx\r\n";
        let bin: &[u8] = &[0, 0, 0, 0];
        let files: &[(&str, &[u8])] = &[
            (r"data\global\excel\PlrMode.txt", txt),
            (r"data\global\excel\plrmode.bin", bin),
        ];
        let install = Install::new("case", &[("d2exp.mpq", files)]);
        let set = install.set();
        assert_eq!(
            read_excel(&set, "plrmode.txt").unwrap(),
            Some(("d2exp.mpq".into(), txt.to_vec()))
        );
        assert_eq!(read_excel(&set, "PLRMODE.BIN").unwrap().unwrap().1, bin);
    }

    #[test]
    fn normal_play_reads_bin_only() {
        let def = schema().table("compcode").unwrap();
        // Only the .txt: no fallback, the missing .bin is fatal.
        let txt: &[u8] = b"code\r\nnil\r\n";
        let only_txt: &[(&str, &[u8])] = &[(r"data\global\excel\compcode.txt", txt)];
        let install = Install::new("txt-only", &[("patch_d2.mpq", only_txt)]);
        match load_bin(&install.set(), def) {
            Err(LoadError::Missing { file }) => {
                assert_eq!(file, r"data\global\excel\compcode.bin")
            }
            other => panic!("{other:?}"),
        }
        // The .bin: count = u32 at 0, records from offset 4.
        let mut bin = 2u32.to_le_bytes().to_vec();
        bin.extend_from_slice(b"nil lit ");
        let both: &[(&str, &[u8])] = &[
            (r"data\global\excel\compcode.txt", txt),
            (r"data\global\excel\compcode.bin", &bin),
        ];
        let install = Install::new("bin", &[("patch_d2.mpq", both)]);
        let t = load_bin(&install.set(), def).unwrap();
        assert_eq!((t.source.as_str(), t.count), ("patch_d2.mpq", 2));
        assert_eq!((t.record(0), t.record(1)), (&b"nil "[..], &b"lit "[..]));
    }

    #[test]
    fn server_only_files_are_fatal() {
        let empty = Install::new("no-server", &[("patch_d2.mpq", &[])]);
        assert!(check_server_files(&empty.set(), "runes").is_ok());
        assert!(check_server_files(&empty.set(), "cubemain").is_ok());
        let cases = [
            ("runes", "runessrv.txt"),
            ("runes", "runessrv.bin"),
            ("runes", "runessrv.xls"),
            ("cubemain", "cubeserver.bin"),
            ("cubemain", "cubeserver.txt"),
        ];
        for (i, (table, file)) in cases.into_iter().enumerate() {
            let path = excel_path(file);
            let files: &[(&str, &[u8])] = &[(&path, b"x")];
            let install = Install::new(&format!("server-{i}"), &[("d2exp.mpq", files)]);
            let set = install.set();
            match check_server_files(&set, table) {
                Err(LoadError::ServerFile { file: f, table: t }) => {
                    assert_eq!((f, t.as_str()), (path.clone(), table))
                }
                other => panic!("{file}: {other:?}"),
            }
            // Only the table the file belongs to checks it.
            let other = if table == "runes" {
                "cubemain"
            } else {
                "runes"
            };
            assert!(check_server_files(&set, other).is_ok());
            assert!(check_server_files(&set, "armor").is_ok());
        }
    }

    #[test]
    fn bin_container_layout() {
        // count 0x0102 = 258, little-endian, then 258 one-byte records.
        let mut data = vec![0x02, 0x01, 0x00, 0x00];
        data.extend((0..258u32).map(|i| i as u8));
        let t = BinTable::parse("t", "p", "t.bin", &data, 1).unwrap();
        assert_eq!(t.count, 258);
        assert_eq!(t.records, &data[4..]);
        // Records packed back to back, no padding or trailer.
        let mut data = 3u32.to_le_bytes().to_vec();
        data.extend_from_slice(b"aaabbbccc");
        let t = BinTable::parse("t", "p", "t.bin", &data, 3).unwrap();
        assert_eq!(t.iter().collect::<Vec<_>>(), [b"aaa", b"bbb", b"ccc"]);
        // No magic: any count bytes are the count; count 0 is 4 bytes.
        let t = BinTable::parse("t", "p", "t.bin", &[0; 4], 52).unwrap();
        assert_eq!((t.count, t.records.len()), (0, 0));
    }

    #[test]
    fn string_tables_load_first() {
        let tbl = empty_tbl();
        let lng = |n: &str| format!(r"data\local\lng\eng\{n}");
        let (s, p, x) = (
            lng("string.tbl"),
            lng("patchstring.tbl"),
            lng("expansionstring.tbl"),
        );
        let two: &[(&str, &[u8])] = &[(&s, &tbl), (&p, &tbl)];
        let classic = Install::new("strings-classic", &[("patch_d2.mpq", two)]);
        let set = classic.set();
        let t = StringTables::load(&set, "eng", false).unwrap();
        assert!(t.base.is_some() && t.patch.is_some() && t.expansion.is_none());
        // expansionstring.tbl is needed only when d2exp.mpq exists.
        assert!(matches!(
            StringTables::load(&set, "eng", true),
            Err(StringsError::Mpq { file, .. }) if file == x
        ));
        // The string tables load before the first excel table: with no
        // compcode.bin, load fails on compcode only once they loaded.
        match load(&set, "eng") {
            Err(LoadError::Missing { file }) => assert_eq!(file, excel_path("compcode.bin")),
            other => panic!("{other:?}"),
        }
        // Without them, load fails on string.tbl although compcode.bin is
        // present.
        let bin = 0u32.to_le_bytes();
        let only_bin: &[(&str, &[u8])] = &[(r"data\global\excel\compcode.bin", &bin)];
        let none = Install::new("strings-none", &[("patch_d2.mpq", only_bin)]);
        assert!(matches!(
            load(&none.set(), "eng"),
            Err(LoadError::Strings(StringsError::Mpq { file, .. })) if file == s
        ));
        // With d2exp.mpq present, expansionstring.tbl is required.
        let lod = Install::new("strings-lod", &[("patch_d2.mpq", two), ("d2exp.mpq", &[])]);
        assert!(matches!(
            load(&lod.set(), "eng"),
            Err(LoadError::Strings(StringsError::Mpq { file, .. })) if file == x
        ));
    }

    #[test]
    fn levels_and_leveldefs() {
        let s = schema();
        let (levels, defs) = (s.table("levels").unwrap(), s.table("leveldefs").unwrap());
        assert_eq!(
            (levels.txt_name.as_str(), levels.record_size),
            ("levels.txt", 544)
        );
        assert_eq!(
            (defs.txt_name.as_str(), defs.record_size),
            ("levels.txt", 156)
        );
        // Normal play reads leveldefs.bin.
        assert_eq!(defs.bin_name, "leveldefs.bin");
        assert!(defs.is_runtime());
        // Different columns of the same rows.
        let cols = |t: &TableDef| {
            t.fields
                .iter()
                .map(|f| f.column.clone())
                .collect::<Vec<_>>()
        };
        assert_ne!(cols(levels), cols(defs));
        // Its count is not kept: row i belongs to levels row i, so the
        // counts must agree.
        let st = StringTables::default();
        let lv = table("levels", 137, 544);
        assert!(post_load_check(
            &table("leveldefs", 137, 156),
            std::slice::from_ref(&lv),
            &st,
            true
        )
        .is_ok());
        assert!(post_load_check(&table("leveldefs", 138, 156), &[lv], &st, true).is_err());
    }

    #[test]
    fn sounds_is_compile_only() {
        let t = schema().table("sounds").unwrap();
        assert_eq!(t.load_step.as_deref(), Some("3.01"));
        assert!(t.live_source.is_none() && !t.is_runtime());
        assert_eq!((t.record_size, t.key_column.as_str()), (2, "Sound"));
        let key = t.field("Sound").unwrap();
        // A name link (key(name16), 2-byte records).
        assert_eq!((key.field_type.id(), key.offset), (17, 0));
        assert!(schema().runtime().all(|d| d.name != "sounds"));
    }

    fn game_set() -> ArchiveSet {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        ArchiveSet::open_dir(dir).expect("archives open")
    }

    /// Loads the full table set from a real install.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn live_bin_set_loads() {
        let set = game_set();
        let data = load(&set, DEFAULT_LANGUAGE).expect("live .bin set loads");
        assert_eq!(data.tables.len(), 73);
        let t = |n: &str| data.table(n).unwrap();
        assert_eq!((t("levels").count, t("leveldefs").count), (137, 137));
        assert_eq!(
            (t("inventory").source.as_str(), t("inventory").count),
            ("d2exp.mpq", 32)
        );
        assert_eq!(t("armor").source, "patch_d2.mpq");
        assert_eq!(t("compcode").record(0), b"nil ");
        assert_eq!(data.code[&CalcBuffer::MissCode].bytes.len(), 196);
        assert_eq!(data.code[&CalcBuffer::SkillsCode].bytes.len(), 5_891);
        assert_eq!(data.code[&CalcBuffer::SkillDescCode].bytes.len(), 4_252);
        assert_eq!(data.code[&CalcBuffer::ItemsCode].bytes.len(), 158);
        assert_eq!(data.code_reports[&CalcBuffer::SkillsCode].paren, 1);
        assert_eq!(data.hitclass.count, 14);
        assert_eq!(data.sounds.records.len(), 4_699);
        assert_eq!(data.soundenviron.records.len(), 50);
        // Every table lives where tables.tsv says.
        for def in schema().runtime() {
            let (archive, _) = def.live_source.as_ref().unwrap();
            assert_eq!(
                t(&def.name).source,
                format!("{archive}.mpq"),
                "{}",
                def.name
            );
        }
        // monstats record 0 TreasureClass1 = 161 + 269.
        let m = t("monstats").record(0);
        assert_eq!(u16::from_le_bytes([m[0x86], m[0x87]]), 430);
        assert_eq!(tc_count(t("itemtypes"), t("treasureclassex")), 1_013);
    }

    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn excel_lookup_order() {
        let set = game_set();
        let src = |f: &str| read_excel(&set, f).unwrap().map(|(s, b)| (s, b.len()));
        assert_eq!(src("inventory.bin").unwrap().0, "d2exp.mpq");
        assert_eq!(src("inventory.txt"), Some(("patch_d2.mpq".into(), 8_843)));
        assert_eq!(src("leveldefs.txt"), None);
        assert_eq!(src("armor.bin"), Some(("patch_d2.mpq".into(), 85_652)));
        let sounds = read_excel(&set, "sounds.bin").unwrap().unwrap().1;
        let t = BinTable::parse("sounds", "p", "sounds.bin", &sounds, 2).unwrap();
        assert_eq!(t.count, 4_699);
        assert!(t
            .iter()
            .enumerate()
            .all(|(i, r)| r == (i as u16).to_le_bytes()));
    }
}

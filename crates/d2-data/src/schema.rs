//! The field lists (`fields.tsv`) and loader calls (`tables.tsv`) that describe
//! every data table of the game.
//!
//! Both files are D2RUST's own tables (they hold no game data), embedded in the
//! crate and parsed on first use by [`schema()`]. The layouts are data-driven: the
//! `.txt` compiler, the `.bin` loader, the fix-ups and the code generator
//! (`codegen`, which writes `tables::generated`) all read column names, types,
//! offsets and links from here.

use std::sync::OnceLock;

/// `tables/data/fields.tsv` (D2RUST's own file; no game data).
pub const FIELDS_TSV: &str = include_str!("../../../tables/data/fields.tsv");
/// `tables/data/tables.tsv` (D2RUST's own file; no game data).
pub const TABLES_TSV: &str = include_str!("../../../tables/data/tables.tsv");

/// Field type IDs 1–26: how one `.txt` column is converted and where it lands in the
/// record. [`FieldType::vocabulary`] gives each type's word as written in
/// `fields.tsv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldType {
    /// `str`: text copied into the record.
    Ascii = 1,
    /// `u32` integer.
    Dword = 2,
    /// `u16` integer.
    Word = 3,
    /// `u8` integer.
    Byte = 4,
    /// `u8` integer (integer rule).
    Unknown1 = 5,
    /// `u8` integer (integer rule).
    Unknown2 = 6,
    /// `str`: text copied into the record.
    Byte2 = 7,
    /// `u32` integer.
    Dword2 = 8,
    /// `code4`: four raw bytes.
    Raw = 9,
    /// `key(code4)`: a four-byte code that is also registered as a key.
    AsciiToCode = 10,
    /// `link32`: a code lookup stored as `u32`.
    Unknown3 = 11,
    /// `key(code1)`: a one-byte code registered as a key.
    Unknown4 = 12,
    /// `link8`: a code lookup stored as `u8`.
    CodeToByte = 13,
    /// `key(code2)`: a two-byte code registered as a key.
    Unknown5 = 14,
    /// `link16`: a code lookup stored as `u16`.
    CodeToWord = 15,
    /// `key(str)`: a text key registered by name.
    Unknown6 = 16,
    /// `key(name16)`: a name key stored as `u16`.
    NameToIndex = 17,
    /// `key(name32)`: a name key stored as `u32`.
    NameToIndex2 = 18,
    /// `link32`: a name lookup stored as `u32`.
    NameToDword = 19,
    /// `link16`: a name lookup stored as `u16`.
    NameToWord = 20,
    /// `link8`: a name lookup stored as `u8`.
    NameToWord2 = 21,
    /// `strkey`: a string key resolved to a `u16` string id.
    KeyToWord = 22,
    /// `cb`: a field callback.
    CustomLink = 23,
    /// `cb`: a field callback.
    Unknown7 = 24,
    /// `cb`: a field callback; with a `calc(...)` link it stores a formula offset as `u32`.
    CalcToDword = 25,
    /// `bit`: one bit of a flags byte.
    Bit = 26,
}

/// The two linker kinds that resolve lookup types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkerKind {
    /// Looks up four-byte item/type codes.
    Code,
    /// Looks up names.
    Name,
}

impl FieldType {
    const ALL: [FieldType; 26] = [
        FieldType::Ascii,
        FieldType::Dword,
        FieldType::Word,
        FieldType::Byte,
        FieldType::Unknown1,
        FieldType::Unknown2,
        FieldType::Byte2,
        FieldType::Dword2,
        FieldType::Raw,
        FieldType::AsciiToCode,
        FieldType::Unknown3,
        FieldType::Unknown4,
        FieldType::CodeToByte,
        FieldType::Unknown5,
        FieldType::CodeToWord,
        FieldType::Unknown6,
        FieldType::NameToIndex,
        FieldType::NameToIndex2,
        FieldType::NameToDword,
        FieldType::NameToWord,
        FieldType::NameToWord2,
        FieldType::KeyToWord,
        FieldType::CustomLink,
        FieldType::Unknown7,
        FieldType::CalcToDword,
        FieldType::Bit,
    ];

    /// The type for ID 1–26.
    pub fn from_id(id: u32) -> Option<FieldType> {
        let i = usize::try_from(id).ok()?.checked_sub(1)?;
        FieldType::ALL.get(i).copied()
    }

    /// The type's numeric ID (1–26).
    pub fn id(self) -> u8 {
        self as u8
    }

    /// Own-key types, which are converted in the first pass.
    pub fn is_own_key(self) -> bool {
        matches!(self.id(), 10 | 12 | 14 | 16 | 17 | 18)
    }

    /// Types that use the integer rule (2–6, 8, 26).
    pub fn is_integer(self) -> bool {
        matches!(self.id(), 2..=6 | 8 | 26)
    }

    /// The linker kind a type 10–21 field needs.
    pub fn linker_kind(self) -> Option<LinkerKind> {
        match self.id() {
            10..=15 => Some(LinkerKind::Code),
            16..=21 => Some(LinkerKind::Name),
            _ => None,
        }
    }

    /// Lookup types (11, 13, 15, 19, 20, 21).
    pub fn is_lookup(self) -> bool {
        matches!(self.id(), 11 | 13 | 15 | 19 | 20 | 21)
    }

    /// Field-callback types (23, 24, 25).
    pub fn is_field_callback(self) -> bool {
        matches!(self.id(), 23..=25)
    }

    /// The type's vocabulary word without its parameter, as in `fields.tsv`.
    pub fn vocabulary(self) -> &'static str {
        match self.id() {
            4..=6 => "u8",
            3 => "u16",
            2 | 8 => "u32",
            26 => "bit",
            1 | 7 => "str",
            9 => "code4",
            10 => "key(code4)",
            12 => "key(code1)",
            14 => "key(code2)",
            16 => "key(str)",
            17 => "key(name16)",
            18 => "key(name32)",
            13 | 21 => "link8",
            15 | 20 => "link16",
            11 | 19 => "link32",
            22 => "strkey",
            _ => "cb",
        }
    }

    /// Bytes the field occupies in the record, for the field-fit check; `len` is the
    /// field's length column. `None` for bit and callback types.
    pub fn width(self, len: u32) -> Option<u32> {
        Some(match self.id() {
            1 | 7 => len + 1,
            16 => len.max(1),
            2 | 8 | 9 | 10 | 11 | 18 | 19 => 4,
            3 | 14 | 15 | 17 | 20 | 22 => 2,
            4 | 5 | 6 | 12 | 13 | 21 => 1,
            _ => return None,
        })
    }
}

/// The four formula code buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CalcBuffer {
    /// `misscode`: missile formulas.
    MissCode,
    /// `skillscode`: skill formulas.
    SkillsCode,
    /// `skilldesccode`: skill description formulas.
    SkillDescCode,
    /// `itemscode`: weapon, armor and misc item formulas.
    ItemsCode,
}

impl CalcBuffer {
    /// All four buffers.
    pub const ALL: [CalcBuffer; 4] = [
        CalcBuffer::MissCode,
        CalcBuffer::SkillsCode,
        CalcBuffer::SkillDescCode,
        CalcBuffer::ItemsCode,
    ];

    /// The buffer's file name without extension (`misscode`, …).
    pub fn name(self) -> &'static str {
        match self {
            CalcBuffer::MissCode => "misscode",
            CalcBuffer::SkillsCode => "skillscode",
            CalcBuffer::SkillDescCode => "skilldesccode",
            CalcBuffer::ItemsCode => "itemscode",
        }
    }

    /// The buffer with this file name, if any.
    pub fn from_name(name: &str) -> Option<CalcBuffer> {
        CalcBuffer::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// The `link` column of `fields.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Link {
    /// Empty: no link (or ID 22, always the string-key resolver).
    None,
    /// A linker: `<table>.<column>`, `items.code`, `@range`,
    /// `@treasureclass`.
    Linker(String),
    /// `calc(<buffer>)`: a formula stored in that code buffer.
    Calc(CalcBuffer),
    /// `param`: a callback that takes a parameter.
    Param,
    /// `cb(<table>.<name>)`: a table-specific callback.
    Table(String),
}

impl Link {
    fn parse(s: &str) -> Result<Link, String> {
        if s.is_empty() {
            return Ok(Link::None);
        }
        if s == "param" {
            return Ok(Link::Param);
        }
        if let Some(inner) = s.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
            return CalcBuffer::from_name(inner)
                .map(Link::Calc)
                .ok_or_else(|| format!("unknown calc buffer `{inner}`"));
        }
        if let Some(inner) = s.strip_prefix("cb(").and_then(|r| r.strip_suffix(')')) {
            return Ok(Link::Table(inner.to_owned()));
        }
        Ok(Link::Linker(s.to_owned()))
    }

    /// The linker name, for linker links.
    pub fn linker(&self) -> Option<&str> {
        match self {
            Link::Linker(name) => Some(name),
            _ => None,
        }
    }
}

/// One field-list entry (a `fields.tsv` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    /// Column name, exact bytes.
    pub column: Vec<u8>,
    /// How the column is converted.
    pub field_type: FieldType,
    /// `str` N, `bit` n, callback argument; else 0.
    pub length: u32,
    /// Byte offset of the field in the record.
    pub offset: u32,
    /// What the field links to, if anything.
    pub link: Link,
}

impl FieldDef {
    /// A field definition from its parts.
    pub fn new(column: &str, field_type: FieldType, length: u32, offset: u32, link: Link) -> Self {
        FieldDef {
            column: column.as_bytes().to_vec(),
            field_type,
            length,
            offset,
            link,
        }
    }

    /// The column name as text (lossily decoded).
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.column).into_owned()
    }

    /// Record bytes this field can write (the bit
    /// byte for `bit(n)`; 4 bytes for `calc`/`param`). Empty for
    /// table-specific callbacks, whose footprint is unknown.
    pub fn footprint(&self) -> std::ops::Range<usize> {
        let o = self.offset as usize;
        match self.field_type {
            FieldType::Bit => {
                let b = o + (self.length as usize >> 3);
                b..b + 1
            }
            t if t.is_field_callback() => match self.link {
                Link::Calc(_) | Link::Param => o..o + 4,
                _ => o..o,
            },
            t => o..o + t.width(self.length).unwrap_or(0) as usize,
        }
    }
}

/// One loader call (a `tables.tsv` row) with its field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDef {
    /// Table name as in `tables.tsv`.
    pub name: String,
    /// `<name>.txt` (`levels.txt` for `leveldefs`); empty for the uncalled
    /// list.
    pub txt_name: String,
    /// Name of the compiled `.bin` file.
    pub bin_name: String,
    /// Bytes per record.
    pub record_size: usize,
    /// Position in the load sequence (`S.NN` for compile-only calls); `None` =
    /// never called.
    pub load_step: Option<String>,
    /// Column holding each row's key.
    pub key_column: String,
    /// `(archive, file)` read in normal play; `None` = compiled only.
    pub live_source: Option<(String, String)>,
    /// Free-form notes, including the `unwritten:` byte ranges.
    pub notes: String,
    /// The field list, in column order.
    pub fields: Vec<FieldDef>,
}

impl TableDef {
    /// A runtime table: its load step has no `.NN` suffix and it has a live `.bin`.
    pub fn is_runtime(&self) -> bool {
        self.live_source.is_some() && self.load_step.as_deref().is_some_and(|s| !s.contains('.'))
    }

    /// Whether the table is compiled in the load sequence.
    pub fn is_called(&self) -> bool {
        self.load_step.is_some()
    }

    /// The field with this column name (exact).
    pub fn field(&self, column: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.column == column.as_bytes())
    }
}

/// All 92 loader calls in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// Every table, in execution order.
    pub tables: Vec<TableDef>,
}

impl Schema {
    /// Parses the two TSV files.
    pub fn parse(tables_tsv: &str, fields_tsv: &str) -> Result<Schema, String> {
        let mut tables = Vec::new();
        let mut lines = tables_tsv.lines();
        let head = lines.next().ok_or("tables.tsv is empty")?;
        if head
            != "table\ttxt_name\tbin_name\trecord_size\tload_step\tkey_column\tlive_source\tnotes"
        {
            return Err(format!("tables.tsv: unexpected header `{head}`"));
        }
        for (n, line) in lines.enumerate() {
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() != 8 {
                return Err(format!("tables.tsv line {}: {} columns", n + 2, c.len()));
            }
            let live_source = match c[6] {
                "-" | "" => None,
                s => {
                    let (archive, file) = s
                        .split_once('/')
                        .ok_or_else(|| format!("tables.tsv line {}: bad live_source", n + 2))?;
                    Some((archive.to_owned(), file.to_owned()))
                }
            };
            tables.push(TableDef {
                name: c[0].to_owned(),
                txt_name: c[1].to_owned(),
                bin_name: c[2].to_owned(),
                record_size: c[3]
                    .parse()
                    .map_err(|e| format!("tables.tsv line {}: record_size: {e}", n + 2))?,
                load_step: (!c[4].is_empty()).then(|| c[4].to_owned()),
                key_column: c[5].to_owned(),
                live_source,
                notes: c[7].to_owned(),
                fields: Vec::new(),
            });
        }

        let mut lines = fields_tsv.lines();
        let head = lines.next().ok_or("fields.tsv is empty")?;
        if head != "table\tseq\tcolumn\ttype_id\ttype\tlength\toffset\tlink" {
            return Err(format!("fields.tsv: unexpected header `{head}`"));
        }
        for (n, line) in lines.enumerate() {
            let at = |m: String| format!("fields.tsv line {}: {m}", n + 2);
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() != 8 {
                return Err(at(format!("{} columns", c.len())));
            }
            let num = |s: &str| s.parse::<u32>().map_err(|e| at(format!("`{s}`: {e}")));
            let table = tables
                .iter_mut()
                .find(|t| t.name == c[0])
                .ok_or_else(|| at(format!("unknown table `{}`", c[0])))?;
            if num(c[1])? as usize != table.fields.len() {
                return Err(at("seq out of order".into()));
            }
            let id = num(c[3])?;
            let field_type = FieldType::from_id(id).ok_or_else(|| at(format!("type id {id}")))?;
            if field_type.vocabulary() != c[4] {
                return Err(at(format!("type `{}` does not match id {id}", c[4])));
            }
            table.fields.push(FieldDef {
                column: c[2].as_bytes().to_vec(),
                field_type,
                length: num(c[5])?,
                offset: num(c[6])?,
                link: Link::parse(c[7]).map_err(at)?,
            });
        }
        Ok(Schema { tables })
    }

    /// The table named `name` (a `tables.tsv` name).
    pub fn table(&self, name: &str) -> Option<&TableDef> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// Tables called by the load, in execution order.
    pub fn called(&self) -> impl Iterator<Item = &TableDef> {
        self.tables.iter().filter(|t| t.is_called())
    }

    /// The runtime tables, in load order.
    pub fn runtime(&self) -> impl Iterator<Item = &TableDef> {
        self.tables.iter().filter(|t| t.is_runtime())
    }
}

/// The embedded schema, parsed on first use.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        Schema::parse(TABLES_TSV, FIELDS_TSV).expect("embedded tables/data/*.tsv must parse")
    })
}

// ------------------------------------------------------ extension fields

/// The value type of an extension column (a column a mod's patch layer
/// adds with `addcol`). Extension columns are never compiled into records:
/// the base record layout stays the same size and order, and mod code reads
/// the values through [`ExtStore::ext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExtType {
    /// A signed 32-bit integer, written as `-?[0-9]+`.
    Int,
    /// Any cell text.
    Text,
}

impl ExtType {
    /// Reads `cell` as a value of this type; `None` when it isn't one. An
    /// empty cell is not a value (it stands for the column's default).
    pub fn value(self, cell: &[u8]) -> Option<ExtValue> {
        match self {
            ExtType::Text => Some(ExtValue::Text(cell.to_vec())),
            ExtType::Int => {
                let digits = cell.strip_prefix(b"-").unwrap_or(cell);
                if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                    return None;
                }
                let n = std::str::from_utf8(cell).ok()?.parse::<i32>().ok()?;
                Some(ExtValue::Int(n))
            }
        }
    }

    /// The type's word in a layer (`int`, `text`).
    pub fn word(self) -> &'static str {
        match self {
            ExtType::Int => "int",
            ExtType::Text => "text",
        }
    }
}

/// One extension cell, typed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExtValue {
    /// An `int` column's value.
    Int(i32),
    /// A `text` column's value.
    Text(Vec<u8>),
}

/// One extension column of a table: its name, type, default and the
/// cell of each record (record `i` is row `i` of the patched table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtColumn {
    /// The header name.
    pub name: Vec<u8>,
    /// The value type.
    pub ty: ExtType,
    /// The default cell text (empty when the layer gave none).
    pub default: Vec<u8>,
    /// The cell text of each record.
    pub cells: Vec<Vec<u8>>,
}

impl ExtColumn {
    /// The value of record `record`: its cell, or the default when the
    /// cell is empty; an `int` column with neither reads 0, a `text`
    /// column reads empty text.
    pub fn get(&self, record: usize) -> Option<ExtValue> {
        let cell = self.cells.get(record)?;
        let text = if cell.is_empty() { &self.default } else { cell };
        Some(self.ty.value(text).unwrap_or(match self.ty {
            ExtType::Int => ExtValue::Int(0),
            ExtType::Text => ExtValue::Text(Vec::new()),
        }))
    }
}

/// The extension columns of a patched data set, by table name (the
/// lowercase txt stem, as `table` statements name it). Empty when no layer
/// used `addcol`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtStore {
    tables: std::collections::BTreeMap<String, Vec<ExtColumn>>,
}

impl ExtStore {
    /// Adds a column to `table`.
    pub fn push(&mut self, table: &str, column: ExtColumn) {
        self.tables
            .entry(table.to_owned())
            .or_default()
            .push(column);
    }

    /// True when no table has an extension column.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// The extension columns of `table`, in `addcol` order.
    pub fn columns(&self, table: &str) -> &[ExtColumn] {
        self.tables.get(table).map_or(&[], Vec::as_slice)
    }

    /// The value of extension column `name` (header names compare without
    /// case, as the compiler binds them) for record `record` of `table`;
    /// `None` when the table has no such column or record.
    pub fn ext(&self, table: &str, record: usize, name: &str) -> Option<ExtValue> {
        self.columns(table)
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name.as_bytes()))?
            .get(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_parses() {
        let s = schema();
        assert_eq!(s.tables.len(), 92);
        assert_eq!(
            s.tables.iter().map(|t| t.fields.len()).sum::<usize>(),
            3_499
        );
        assert_eq!(s.called().count(), 91);
        assert_eq!(s.runtime().count(), 73);
        assert_eq!(
            s.tables.iter().map(|t| t.fields.len()).max(),
            Some(253),
            "longest list (monstats)"
        );
    }

    /// Runtime table order, record sizes and first/last steps.
    #[test]
    fn runtime_tables_follow_loading_md() {
        let s = schema();
        let names: Vec<&str> = s.runtime().map(|t| t.name.as_str()).collect();
        assert_eq!(names[0], "compcode");
        assert_eq!(names[72], "difficultylevels");
        assert_eq!(s.table("levels").unwrap().record_size, 544);
        assert_eq!(s.table("leveldefs").unwrap().record_size, 156);
        assert_eq!(s.table("leveldefs").unwrap().txt_name, "levels.txt");
        assert_eq!(s.table("lvltypes").unwrap().record_size, 1928);
        let live_in_x = s
            .runtime()
            .filter(|t| t.live_source.as_ref().unwrap().0 == "d2exp")
            .count();
        assert_eq!(live_in_x, 17);
    }

    /// Spot checks on the embedded field lists.
    #[test]
    fn schema_vectors() {
        let s = schema();
        let f = &s.table("skills").unwrap().fields[60];
        assert_eq!(f.column, b"pettype");
        assert_eq!(f.field_type, FieldType::NameToWord2);
        assert_eq!((f.offset, f.footprint()), (190, 190..191));
        assert_eq!(f.link, Link::Linker("pettype.pet type".into()));
        let f = &s.table("runes").unwrap().fields[13];
        assert_eq!(f.column, b"rune1");
        assert_eq!(f.field_type, FieldType::Unknown3);
        assert_eq!(f.link, Link::Linker("items.code".into()));
    }

    /// Bytes no field writes, as listed in the `tables.tsv`
    /// notes (`unwritten: a-b,c`), equal the bytes outside every field
    /// footprint.
    #[test]
    fn footprints_match_unwritten_notes() {
        let mut total = 0;
        let mut tables = 0;
        for t in schema().tables.iter() {
            let mut covered = vec![false; t.record_size];
            for f in &t.fields {
                for o in f.footprint() {
                    covered[o] = true;
                }
            }
            let ours: Vec<usize> = (0..t.record_size).filter(|&o| !covered[o]).collect();
            let listed: Vec<usize> = t
                .notes
                .split("; ")
                .find_map(|n| n.strip_prefix("unwritten: "))
                .map(|ranges| {
                    ranges
                        .split(',')
                        .flat_map(|r| {
                            let (a, b) = r.split_once('-').unwrap_or((r, r));
                            a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap()
                        })
                        .collect()
                })
                .unwrap_or_default();
            assert_eq!(ours, listed, "{}", t.name);
            if !ours.is_empty() {
                tables += 1;
            }
            total += ours.len();
        }
        assert_eq!((total, tables), (1_702, 55));
    }

    /// `(S, NN)` of a `load_step` (`S` or `S.NN`).
    fn step(s: &str) -> (u32, u32) {
        let (a, b) = s.split_once('.').unwrap_or((s, "0"));
        (a.parse().unwrap(), b.parse().unwrap())
    }

    /// 92 loader calls; the 91 reachable ones are in load order and one is never
    /// called.
    #[test]
    fn execution_order() {
        let s = schema();
        assert_eq!(s.tables.len(), 92);
        let steps: Vec<(u32, u32)> = s
            .called()
            .map(|t| step(t.load_step.as_deref().unwrap()))
            .collect();
        assert_eq!(steps.len(), 91);
        assert!(steps.windows(2).all(|w| w[0] < w[1]), "rows in step order");
        let runtime: Vec<u32> = steps.iter().filter(|s| s.1 == 0).map(|s| s.0).collect();
        assert_eq!(runtime, (1..=73).collect::<Vec<_>>());
        let uncalled: Vec<&TableDef> = s.tables.iter().filter(|t| !t.is_called()).collect();
        assert_eq!(uncalled.len(), 1);
        assert_eq!(uncalled[0].name, "unused_653db0");
        assert!(uncalled[0].notes.starts_with("call 0x653DFF;"));
        // Every row is one call site.
        let mut sites: Vec<&str> = s
            .tables
            .iter()
            .map(|t| t.notes.split(';').next().unwrap())
            .collect();
        assert!(sites.iter().all(|c| c.starts_with("call 0x")));
        sites.sort();
        sites.dedup();
        assert_eq!(sites.len(), 92);
    }

    /// Names, linker names and callbacks as derived from the field lists.
    #[test]
    fn provenance_of_names_and_links() {
        let s = schema();
        // 3,499 entries plus one terminator per list = 3,591 (the
        // independent extraction's count); no terminator is a row.
        let fields: usize = s.tables.iter().map(|t| t.fields.len()).sum();
        assert_eq!(fields + s.tables.len(), 3_591);
        // Linker names: `table.column` of the key entry filling it.
        for t in &s.tables {
            for f in &t.fields {
                let Link::Linker(l) = &f.link else { continue };
                if l.starts_with('@') {
                    assert!(l == "@range" || l == "@treasureclass", "{l}");
                    continue;
                }
                let (owner, column) = l.split_once('.').unwrap();
                let owners: &[&str] = if l == "items.code" {
                    &["weapons", "armor", "misc"]
                } else {
                    &[owner]
                };
                for o in owners {
                    let o = s.table(o).unwrap_or_else(|| panic!("{l}"));
                    let key = o.field(&o.key_column).unwrap();
                    assert!(key.field_type.is_own_key(), "{l}");
                    if l != "items.code" {
                        assert_eq!(o.key_column, column, "{l}");
                        assert_eq!(key.link, Link::Linker(l.clone()), "{l}");
                    }
                }
            }
        }
        // Callbacks by name.
        let mut cbs: Vec<String> = s
            .tables
            .iter()
            .flat_map(|t| &t.fields)
            .filter_map(|f| match &f.link {
                Link::Calc(b) => Some(format!("calc({})", b.name())),
                Link::Param => Some("param".into()),
                Link::Table(t) => Some(format!("cb({t})")),
                _ => None,
            })
            .collect();
        cbs.sort();
        cbs.dedup();
        assert_eq!(
            cbs,
            [
                "calc(itemscode)",
                "calc(misscode)",
                "calc(skilldesccode)",
                "calc(skillscode)",
                "cb(cubemain.input)",
                "cb(cubemain.output)",
                "cb(cubemain.param)",
                "cb(monpreset.place)",
                "cb(monstats.skillmode)",
                "cb(monstats2.composit)",
                "param",
            ]
        );
        // The 7 types that differ from older public field lists keep the shipped types.
        let id = |t: &str, c: &str| s.table(t).unwrap().field(c).unwrap().field_type.id();
        assert_eq!(id("monplace", "code"), 17);
        assert_eq!(id("gamble", "code"), 9);
        for t in [
            "uniquetitle",
            "uniqueprefix",
            "uniquesuffix",
            "uniqueappellation",
            "unused_653db0",
        ] {
            assert_eq!(id(t, "Name"), 22, "{t}");
        }
    }

    #[test]
    fn list_edge_cases() {
        let s = schema();
        for t in &s.tables {
            let names: Vec<Vec<u8>> = t
                .fields
                .iter()
                .map(|f| f.column.to_ascii_lowercase())
                .collect();
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), names.len(), "{}: duplicate column", t.name);
        }
        let longest = s.tables.iter().max_by_key(|t| t.fields.len()).unwrap();
        assert_eq!(
            (longest.name.as_str(), longest.fields.len()),
            ("monstats", 253)
        );
        // The camt overlap and a NUL spill are kept as shipped.
        let levels = s.table("levels").unwrap();
        for c in ["camt1", "camt2", "camt3", "camt4"] {
            assert_eq!(levels.field(c).unwrap().offset, 220, "{c}");
        }
        let pet = s.table("pettype").unwrap();
        let base = pet.field("baseicon").unwrap();
        assert_eq!(
            base.footprint().end,
            pet.field("micon1").unwrap().offset as usize + 1
        );
    }

    /// A table compiles only after every table whose
    /// link it reads (or is that table); hand-built linkers exist first.
    #[test]
    fn links_point_to_earlier_tables() {
        let s = schema();
        let order: Vec<&str> = s.called().map(|t| t.name.as_str()).collect();
        let pos = |n: &str| {
            order
                .iter()
                .position(|&o| o == n)
                .unwrap_or_else(|| panic!("{n}"))
        };
        for t in s.called() {
            let me = pos(&t.name);
            for f in &t.fields {
                let Link::Linker(l) = &f.link else { continue };
                let owner = match l.as_str() {
                    "@range" => "skills", // built inside step 10, before skills compiles
                    "@treasureclass" => "treasureclassex",
                    "items.code" => "weapons",
                    _ => l.split_once('.').unwrap().0,
                };
                let at = pos(owner);
                if l.starts_with('@') && l != "@range" {
                    assert!(at < me, "{}: {l}", t.name);
                } else {
                    assert!(at <= me, "{}: {l}", t.name);
                }
            }
        }
    }

    #[test]
    fn type_ids_round_trip() {
        for id in 1..=26 {
            assert_eq!(u32::from(FieldType::from_id(id).unwrap().id()), id);
        }
        assert!(FieldType::from_id(0).is_none());
        assert!(FieldType::from_id(27).is_none());
    }
}

//! Mod patch layers (`d2patch 1`): row and cell statements applied in
//! stack order to the player's own excel `.txt` cells, then compiled by the
//! txt compiler. Layers never touch `.bin` files or records. The server and
//! the `data-tool patch` command build a [`PatchData`] from the base tables,
//! run a stack over it and read back the findings and the patched live set.
//!
//! - `syntax`: stack and layer files (S and P findings).
//! - `apply`: selectors, columns and the stack (A and N findings).
//! - `diff`: an edited `.txt` → a canonical layer (D).
//! - `check`: compile the patched cells and compare (C).
//!
//! Statements: `table`, `set`, `check`, `add`, `addcol`, `stradd` and
//! `strset`.
//! `addcol <name> <type> [<default>]` appends an extension column to the
//! current table, so a mod game can carry its own data on base tables
//! (an item stat's new property, a level's new flag). The type is `int`
//! (`-?[0-9]+`, 32-bit) or `text`; every row, and every later `add`
//! without `like`, starts with the default (empty when none, which reads
//! as 0 or empty text). The name must be new to the header and to every
//! field the compiler binds on that table (A16), and `int` defaults and
//! `set` values must be integers (A17). No compiled field binds an
//! extension column, so compiled records keep their size and bytes; mod
//! code reads the values through [`crate::schema::ExtStore`]
//! ([`PatchedCompile::ext`], [`PatchData::ext_store`]). A layer without
//! `addcol` patches exactly as before, and the format stays `d2patch 1`.
//!
//! `stradd <key> <text>` adds a string and `strset <key> <text>` changes
//! one, by key; they stand outside table zones and are kept in
//! [`PatchData::strings`] until the string tables are at hand
//! ([`apply_strings`]): an added key gets the next id from
//! [`crate::strings::MOD_BASE`] and binds in `strkey` cells like any
//! other, a changed key keeps its id (A18 key exists, A19 no such key,
//! A20 no id left). String tables themselves are never a `table`.
//!
//! Everything here is pure: no files, clock, environment, hash-map order
//! or floats. Tools do the I/O (`data-tool patch`).

mod apply;
mod check;
mod diff;
#[cfg(test)]
mod gaps_numbered_tests;
#[cfg(test)]
mod robust_tests;
mod syntax;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c2fmt;

use std::fmt;

use sha2::{Digest, Sha256};

use crate::compile::code4;
use crate::schema::{schema, ExtColumn, ExtStore, ExtType, FieldType, Schema};
use crate::strings::{StringEdit, StringEditError, StringTables};
use crate::txt::{bind, TxtTable};

pub use apply::apply_stack;
pub use check::{compile_patched, PatchedCompile};
pub use diff::{diff_tables, DiffError};
pub use syntax::{
    load_stack, parse_layer, parse_stack, Layer, Sel, StackEntry, Stmt, StmtLine, Tok,
};

/// The layer and stack format version this engine reads.
pub const VERSION: u32 = 1;

// ------------------------------------------------------------- findings

/// The finding codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    S01,
    S02,
    S03,
    S04,
    S05,
    S06,
    S07,
    P01,
    P02,
    P03,
    P04,
    P05,
    P06,
    P07,
    P08,
    P09,
    P10,
    P11,
    P12,
    B01,
    B02,
    B03,
    A01,
    A02,
    A03,
    A04,
    A05,
    A06,
    A07,
    A08,
    A09,
    A10,
    A11,
    A12,
    A13,
    A14,
    A15,
    A16,
    A17,
    A18,
    A19,
    A20,
    N01,
    N02,
    N03,
    N04,
    C01,
    C02,
    C03,
    C04,
    D01,
    D02,
    D03,
    D04,
    D05,
    D06,
}

impl Code {
    /// Notes (N) and tolerated compiler diagnostics (C04, a mod game's
    /// data) are not errors; everything else is.
    pub fn is_error(self) -> bool {
        !matches!(
            self,
            Code::N01 | Code::N02 | Code::N03 | Code::N04 | Code::C04
        )
    }

    /// The condition, in words (not normative).
    pub fn describe(self) -> &'static str {
        match self {
            Code::S01 => "line 1 is not `d2stack 1`",
            Code::S02 => "unsupported stack version",
            Code::S03 => "not a `layer <path>` line",
            Code::S04 => "invalid layer path",
            Code::S05 => "layer listed twice",
            Code::S06 => "layer file unreadable",
            Code::S07 => "more than 1,024 layers",
            Code::P01 => "byte outside 0x20-0x7E, LF, CR LF",
            Code::P02 => "line 1 is not `d2patch 1`",
            Code::P03 => "unsupported layer version",
            Code::P04 => "unknown statement",
            Code::P05 => "missing, extra or unexpected token",
            Code::P06 => "unterminated bracket",
            Code::P07 => "invalid row index",
            Code::P08 => "invalid table name",
            Code::P09 => "statement before the first `table`",
            Code::P10 => "token not followed by a space or line end",
            Code::P11 => "invalid pin",
            Code::P12 => "file, line or token too long",
            Code::B01 => "base table absent",
            Code::B02 => "base table unreadable",
            Code::B03 => "key column not addressable",
            Code::A01 => "not a patchable table",
            Code::A02 => "column not found or ambiguous",
            Code::A03 => "no such row",
            Code::A04 => "several rows have this key",
            Code::A05 => "row label differs",
            Code::A06 => "cell differs from the stated value",
            Code::A07 => "old and new values are equal",
            Code::A08 => "cell already written by this layer",
            Code::A09 => "add index is not the row count",
            Code::A10 => "table has a fixed row count",
            Code::A11 => "`Expansion` in column 0",
            Code::A12 => "empty or duplicate key",
            Code::A13 => "template pin differs",
            Code::A14 => "key selector renamed in this layer",
            Code::A15 => "row is a template in this layer",
            Code::A16 => "column already exists",
            Code::A17 => "value is not of the column's type",
            Code::A18 => "string key already exists",
            Code::A19 => "no string has this key",
            Code::A20 => "no string id left",
            Code::N01 => "replaces an earlier layer's value",
            Code::N02 => "no list binds this column",
            Code::N03 => "statements skipped",
            Code::N04 => "layers not applied",
            Code::C01 => "compile failed",
            Code::C02 => "new compiler diagnostic",
            Code::C03 => "live-set check failed",
            Code::C04 => "compiler diagnostic tolerated (mod data)",
            Code::D01 => "rows removed",
            Code::D02 => "header changed",
            Code::D03 => "value not expressible",
            Code::D04 => "edited file unreadable",
            Code::D05 => "rows inserted, deleted or sorted",
            Code::D06 => "row count or key rule broken",
        }
    }

    fn class(self) -> u8 {
        match format!("{self:?}").as_bytes()[0] {
            b'S' => 0,
            b'P' => 1,
            b'B' => 2,
            b'A' | b'N' => 3,
            b'C' => 4,
            _ => 5,
        }
    }
}

/// One report entry. Lines and columns are 1-based, 0 when none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The finding code.
    pub code: Code,
    /// Layer stack path, stack file, or `base:<archive>:<path>`.
    pub file: String,
    /// 1-based line, 0 when none.
    pub line: usize,
    /// 1-based column, 0 when none.
    pub col: usize,
    /// Name of the table the finding is about.
    pub table: Option<String>,
    /// (index, key).
    pub row: Option<(usize, Vec<u8>)>,
    /// Canonical column reference.
    pub column: Option<Vec<u8>>,
    /// The value the statement expected.
    pub expected: Option<Vec<u8>>,
    /// The value actually found.
    pub found: Option<Vec<u8>>,
    /// `base line N` or `layer:line`.
    pub writer: Option<String>,
    /// Other places involved (writers, rows), as text.
    pub related: Vec<String>,
    /// Free wording (not normative).
    pub detail: String,
    /// Stack position: 0 the stack file, `k + 1` layer `k` (ordering only).
    pub pos: usize,
}

impl Finding {
    /// An empty finding of `code` at `file`:`line`:`col`.
    pub fn new(code: Code, file: &str, line: usize, col: usize) -> Finding {
        Finding {
            code,
            file: file.to_owned(),
            line,
            col,
            table: None,
            row: None,
            column: None,
            expected: None,
            found: None,
            writer: None,
            related: Vec::new(),
            detail: String::new(),
            pos: 0,
        }
    }

    fn at_pos(mut self, pos: usize) -> Finding {
        self.pos = pos;
        self
    }

    fn detail(mut self, d: impl Into<String>) -> Finding {
        self.detail = d.into();
        self
    }

    fn sort_key(&self) -> (u8, usize, usize, usize, Code, String, usize, Vec<u8>) {
        let class = self.code.class();
        let row = self.row.as_ref().map_or(0, |r| r.0);
        let column = self.column.clone().unwrap_or_default();
        let table = self.table.clone().unwrap_or_default();
        match class {
            0 | 1 => (
                class, self.pos, self.line, self.col, self.code, table, 0, column,
            ),
            3 if self.code == Code::N04 => (class, usize::MAX, 0, 0, self.code, table, 0, column),
            3 => (
                class, self.pos, self.line, self.col, self.code, table, 0, column,
            ),
            // B (table) and C (table, row, column): never by code.
            _ => (class, 0, 0, 0, Code::C01, table, row, column),
        }
    }
}

/// `[bytes]`, lossy.
fn bracket(v: &[u8]) -> String {
    format!("[{}]", String::from_utf8_lossy(v))
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = if self.code.is_error() {
            "error"
        } else {
            "note"
        };
        write!(
            f,
            "{severity}[{:?}] {}:{}:{}:",
            self.code, self.file, self.line, self.col
        )?;
        if let Some(t) = &self.table {
            write!(f, " {t}")?;
        }
        if let Some((i, key)) = &self.row {
            write!(
                f,
                " {} (#{i})",
                String::from_utf8_lossy(&canonical_token(key))
            )?;
        }
        if let Some(c) = &self.column {
            write!(f, " {}", String::from_utf8_lossy(c))?;
        }
        if self.table.is_some() || self.row.is_some() || self.column.is_some() {
            write!(f, ":")?;
        }
        let mut parts = Vec::new();
        parts.push(if self.detail.is_empty() {
            self.code.describe().to_owned()
        } else {
            self.detail.clone()
        });
        if let Some(e) = &self.expected {
            parts.push(format!("expected {}", bracket(e)));
        }
        if let Some(v) = &self.found {
            parts.push(format!("found {}", bracket(v)));
        }
        if let Some(w) = &self.writer {
            parts.push(format!("writer {w}"));
        }
        if !self.related.is_empty() {
            parts.push(format!("related: {}", self.related.join(", ")));
        }
        write!(f, " {}", parts.join("; "))
    }
}

/// Sorts findings in report order.
pub fn sort_report(findings: &mut [Finding]) {
    findings.sort_by_cached_key(Finding::sort_key);
}

/// Whether any finding is an error.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.code.is_error())
}

// ----------------------------------------------------------- table rules

/// How keys compare: as the code or name linker does, or exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// First 4 bytes, space-padded, case-sensitive.
    Code,
    /// First 31 bytes, ASCII-lowercased.
    Name,
    /// Tables without a linker key (`multi`).
    Exact,
}

impl KeyKind {
    /// The normalized key of a cell.
    pub fn normalize(self, cell: &[u8]) -> Vec<u8> {
        match self {
            KeyKind::Code => code4(cell).to_vec(),
            KeyKind::Name => cell[..cell.len().min(31)].to_ascii_lowercase(),
            KeyKind::Exact => cell.to_vec(),
        }
    }
}

/// The patch rules of one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRules {
    /// Lowercase stem of the txt name.
    pub name: String,
    /// File name of the table's `.txt`.
    pub txt_name: String,
    /// The key field's column name; `None` = column 0.
    pub key_field: Option<Vec<u8>>,
    /// How keys of this table compare.
    pub kind: KeyKind,
    /// `unique` (else `multi`).
    pub unique: bool,
    /// `fixed` rows (else `append`).
    pub fixed: bool,
    /// Uniqueness scope: the linker the key fills, or the table name.
    pub scope: String,
    /// Field column names of each list compiled from this txt.
    pub lists: Vec<Vec<Vec<u8>>>,
}

/// Tables whose row count the loader's record-count check pins.
const FIXED: [&str; 7] = [
    "arena",
    "armtype",
    "belts",
    "composit",
    "difficultylevels",
    "experience",
    "inventory",
];

/// The patchable tables of a schema: one per distinct `txt_name` of a
/// called list, in first-call order.
pub fn rules_from_schema(schema: &Schema) -> Vec<TableRules> {
    let mut out: Vec<TableRules> = Vec::new();
    for def in schema.called() {
        let lists: Vec<Vec<u8>> = def.fields.iter().map(|f| f.column.clone()).collect();
        if let Some(r) = out.iter_mut().find(|r| r.txt_name == def.txt_name) {
            r.lists.push(lists);
            continue;
        }
        let name = def
            .txt_name
            .strip_suffix(".txt")
            .unwrap_or(&def.txt_name)
            .to_ascii_lowercase();
        out.push(TableRules {
            name: name.clone(),
            txt_name: def.txt_name.clone(),
            key_field: None,
            kind: KeyKind::Exact,
            unique: false,
            fixed: FIXED.contains(&name.as_str()),
            scope: name,
            lists: vec![lists],
        });
    }
    // The key: the first list with a key(code4)/key(name16)/key(name32)
    // entry named by its `key_column`.
    for r in &mut out {
        let key = schema
            .called()
            .filter(|d| d.txt_name == r.txt_name && !d.key_column.is_empty())
            .find_map(|d| {
                let f = d
                    .fields
                    .iter()
                    .find(|f| f.column.eq_ignore_ascii_case(d.key_column.as_bytes()))?;
                let kind = match f.field_type {
                    FieldType::AsciiToCode => KeyKind::Code,
                    FieldType::NameToIndex | FieldType::NameToIndex2 => KeyKind::Name,
                    _ => return None,
                };
                Some((f.column.clone(), kind, f.link.linker()?.to_owned()))
            });
        if let Some((column, kind, scope)) = key {
            r.key_field = Some(column);
            r.kind = kind;
            r.scope = scope;
            r.unique = r.name != "monseq";
        } else if r.name == "treasureclassex" {
            r.kind = KeyKind::Name;
            r.unique = true;
        }
    }
    out
}

/// The 85 patchable tables of the game's data set.
pub fn rules() -> Vec<TableRules> {
    rules_from_schema(schema())
}

// --------------------------------------------------------------- cells

/// A layer location: stack position (0-based layer index) and line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Loc {
    /// Layer index in the stack.
    pub layer: usize,
    /// 1-based line in the layer file.
    pub line: usize,
}

/// Where a row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Base file line.
    Base(usize),
    Added(Loc),
}

/// Who last wrote a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writer {
    Base,
    Add(Loc),
    Copy(Loc),
    Set(Loc),
}

impl Writer {
    /// The layer location of this writer; `None` for the base file.
    pub fn loc(self) -> Option<Loc> {
        match self {
            Writer::Base => None,
            Writer::Add(l) | Writer::Copy(l) | Writer::Set(l) => Some(l),
        }
    }
}

/// One table row: its cells and who wrote each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Where the row came from.
    pub origin: Origin,
    /// Cell texts, one per column.
    pub cells: Vec<Vec<u8>>,
    /// Who last wrote each cell, one per column.
    pub writers: Vec<Writer>,
}

/// One table's cells with provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchTable {
    /// The patch rules of this table.
    pub rules: TableRules,
    /// Archive the base came from.
    pub source: String,
    /// Header cell texts.
    pub header: Vec<Vec<u8>>,
    /// The data rows.
    pub rows: Vec<Row>,
    /// Index of the key column.
    pub key_col: usize,
    /// Per column: some list of this txt binds it.
    pub bound: Vec<bool>,
    /// Extension columns added by `addcol`, in order (empty for a base
    /// table).
    pub ext: Vec<AddedColumn>,
}

/// An extension column a layer appended with `addcol`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedColumn {
    /// The header name.
    pub name: Vec<u8>,
    /// The value type.
    pub ty: ExtType,
    /// The default cell text (empty when none).
    pub default: Vec<u8>,
    /// Its column index in the header.
    pub column: usize,
    /// The `addcol` statement.
    pub loc: Loc,
}

impl PatchTable {
    /// Builds a table from its base file (B02, B03 on failure).
    pub fn from_base(
        rules: &TableRules,
        source: &str,
        bytes: &[u8],
    ) -> Result<PatchTable, Box<Finding>> {
        let file = base_file(source, &rules.txt_name);
        let txt = TxtTable::parse(&file, bytes).map_err(|e| {
            let mut f = Finding::new(Code::B02, &file, e.line.unwrap_or(0), 0).detail(format!(
                "{:?} {}",
                e.code,
                e.code.describe()
            ));
            f.table = Some(rules.name.clone());
            Box::new(f)
        })?;
        let rows = txt
            .records
            .iter()
            .map(|r| Row {
                origin: Origin::Base(r.line),
                writers: vec![Writer::Base; r.cells.len()],
                cells: r.cells.clone(),
            })
            .collect();
        PatchTable::new(rules, source, txt.header, rows)
    }

    /// A table from a header and rows (tests, fixtures).
    pub fn new(
        rules: &TableRules,
        source: &str,
        header: Vec<Vec<u8>>,
        rows: Vec<Row>,
    ) -> Result<PatchTable, Box<Finding>> {
        let b03 = |detail: &str| {
            let mut f = Finding::new(Code::B03, &base_file(source, &rules.txt_name), 0, 0)
                .detail(detail.to_owned());
            f.table = Some(rules.name.clone());
            Box::new(f)
        };
        let mut bound = vec![false; header.len()];
        let mut key_col = 0;
        for list in &rules.lists {
            let b = bind(&header, list);
            for (c, f) in b.column_field.iter().enumerate() {
                if f.is_some() {
                    bound[c] = true;
                }
            }
        }
        if let Some(key) = &rules.key_field {
            let list = rules
                .lists
                .iter()
                .find(|l| l.iter().any(|n| n == key))
                .ok_or_else(|| b03("key field in no list"))?;
            let b = bind(&header, list);
            let f = list.iter().position(|n| n == key).expect("found above");
            key_col = b.field_column[f].ok_or_else(|| b03("the key entry binds no column"))?;
            let name = &header[key_col];
            if header
                .iter()
                .enumerate()
                .any(|(c, h)| c != key_col && h.eq_ignore_ascii_case(name))
            {
                return Err(b03("another column's name equals the key column's"));
            }
        }
        Ok(PatchTable {
            rules: rules.clone(),
            source: source.to_owned(),
            header,
            rows,
            key_col,
            bound,
            ext: Vec::new(),
        })
    }

    /// The table's lowercase name.
    pub fn name(&self) -> &str {
        &self.rules.name
    }

    /// The key cell of row `row`.
    pub fn key(&self, row: usize) -> &[u8] {
        &self.rows[row].cells[self.key_col]
    }

    /// The canonical reference of column `c`: the name if unique, else
    /// `name@k`.
    pub fn canonical(&self, c: usize) -> Vec<u8> {
        canonical_column(&self.header, c)
    }

    /// `render(T)`: header and rows, TAB-joined, CR LF lines.
    pub fn render(&self) -> Vec<u8> {
        render(&self.header, self.rows.iter().map(|r| &r.cells[..]))
    }

    /// Hex SHA-256 of `render(T)`.
    pub fn digest(&self) -> String {
        hex(&Sha256::digest(self.render()))
    }

    /// The pin of row `r`.
    pub fn pin(&self, r: usize) -> String {
        pin(&self.header, &self.rows[r].cells)
    }

    /// Rows keyed exactly `key`.
    pub fn rows_keyed(&self, key: &[u8]) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&i| self.key(i) == key)
            .collect()
    }
}

/// `base:<archive>:<path>`.
pub fn base_file(source: &str, txt_name: &str) -> String {
    format!("base:{source}:data\\global\\excel\\{txt_name}")
}

pub(crate) fn canonical_column(header: &[Vec<u8>], c: usize) -> Vec<u8> {
    let name = &header[c];
    let same: Vec<usize> = (0..header.len()).filter(|&i| &header[i] == name).collect();
    if same.len() == 1 {
        return name.clone();
    }
    let k = same.iter().position(|&i| i == c).expect("c is in same") + 1;
    let mut out = name.clone();
    out.extend_from_slice(format!("@{k}").as_bytes());
    out
}

pub(crate) fn render<'a>(header: &[Vec<u8>], rows: impl Iterator<Item = &'a [Vec<u8>]>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut line = |cells: &[Vec<u8>]| {
        out.extend_from_slice(&cells.join(&b'\t'));
        out.extend_from_slice(b"\r\n");
    };
    line(header);
    for r in rows {
        line(r);
    }
    out
}

pub(crate) fn pin(header: &[Vec<u8>], cells: &[Vec<u8>]) -> String {
    let mut h = Sha256::new();
    for (c, cell) in cells.iter().enumerate() {
        if cell.is_empty() {
            continue;
        }
        h.update(canonical_column(header, c));
        h.update(b"\t");
        h.update(cell);
        h.update(b"\n");
    }
    format!("sha:{}", &hex(&h.finalize())[..16])
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The canonical token for a value: bare if non-empty, bytes
/// 0x21–0x7E without `[` `]`, no leading `#`, not `->` or `like`; else
/// bracketed.
pub fn canonical_token(v: &[u8]) -> Vec<u8> {
    let bare = !v.is_empty()
        && v.iter()
            .all(|&b| (0x21..=0x7E).contains(&b) && b != b'[' && b != b']')
        && v[0] != b'#'
        && v != b"->"
        && v != b"like";
    if bare {
        v.to_vec()
    } else {
        let mut out = vec![b'['];
        out.extend_from_slice(v);
        out.push(b']');
        out
    }
}

// ----------------------------------------------------------------- data

/// A change-log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The statement's location.
    pub loc: Loc,
    /// Table index in [`PatchData::tables`].
    pub table: usize,
    /// Row index in the table.
    pub row: usize,
    /// Row key when the statement ran.
    pub key: Vec<u8>,
    /// `None` for an `add`.
    pub set: Option<SetChange>,
}

/// The cell change made by a `set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetChange {
    /// Column index.
    pub column: usize,
    /// Cell text before.
    pub old: Vec<u8>,
    /// Cell text after.
    pub new: Vec<u8>,
}

/// The patchable tables with provenance, the layer names and the change
/// log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchData {
    /// The patchable tables.
    pub tables: Vec<PatchTable>,
    /// Stack paths of the applied layers, by stack position.
    pub layers: Vec<String>,
    /// Every applied `add` and `set`, in order.
    pub changes: Vec<Change>,
    /// Every `stradd` and `strset`, in stack order. They apply over the
    /// string tables where those are loaded ([`apply_strings`]).
    pub strings: Vec<StrEdit>,
}

/// A `stradd` / `strset` a layer wrote, with where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrEdit {
    /// The edit.
    pub edit: StringEdit,
    /// The layer's stack path.
    pub path: String,
    /// 1-based stack position of the layer.
    pub pos: usize,
    /// 1-based line of the statement.
    pub line: usize,
    /// 1-based column of the key token.
    pub col: usize,
}

/// Applies the layers' string edits over `strings`, in stack order. A
/// refused edit is an error finding: A18 `stradd` of a key that exists,
/// A19 `strset` of a key no string has, A20 no id left for a new key.
/// Later edits still apply, so one run reports every refusal.
pub fn apply_strings(strings: &mut StringTables, edits: &[StrEdit]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for e in edits {
        if let Err(err) = strings.apply(&e.edit) {
            let code = match err {
                StringEditError::KeyExists => Code::A18,
                // The parser refuses empty keys (P05); none reaches here.
                StringEditError::NoSuchKey | StringEditError::EmptyKey => Code::A19,
                StringEditError::Full => Code::A20,
            };
            findings.push(Finding::new(code, &e.path, e.line, e.col).at_pos(e.pos));
        }
    }
    findings
}

/// A source of base `.txt` files: `(archive, bytes)` for an excel file
/// name, `None` when absent.
pub type BaseReader<'a> = dyn FnMut(&str) -> Result<Option<(String, Vec<u8>)>, String> + 'a;

impl PatchData {
    /// Reads every table's base: B01 absent, B02 reader error, B03.
    pub fn from_base(
        rules: &[TableRules],
        read: &mut BaseReader<'_>,
    ) -> Result<Self, Vec<Finding>> {
        let mut tables = Vec::new();
        let mut errors = Vec::new();
        for r in rules {
            let file = format!("data\\global\\excel\\{}", r.txt_name);
            let mut fail = |code, detail: String| {
                let mut f = Finding::new(code, &file, 0, 0).detail(detail);
                f.table = Some(r.name.clone());
                errors.push(f);
            };
            match read(&r.txt_name) {
                Ok(None) => fail(Code::B01, "not found in any archive".into()),
                Err(e) => fail(Code::B02, e),
                Ok(Some((source, bytes))) => match PatchTable::from_base(r, &source, &bytes) {
                    Ok(t) => tables.push(t),
                    Err(f) => errors.push(*f),
                },
            }
        }
        if errors.is_empty() {
            Ok(PatchData::from_tables(tables))
        } else {
            sort_report(&mut errors);
            Err(errors)
        }
    }

    /// Wraps `tables` with no layers applied and an empty change log.
    pub fn from_tables(tables: Vec<PatchTable>) -> PatchData {
        PatchData {
            tables,
            layers: Vec::new(),
            changes: Vec::new(),
            strings: Vec::new(),
        }
    }

    /// The table called `name`, if any.
    pub fn table(&self, name: &str) -> Option<&PatchTable> {
        self.tables.iter().find(|t| t.name() == name)
    }

    /// The extension columns of every table, with each record's cell.
    pub fn ext_store(&self) -> ExtStore {
        let mut store = ExtStore::default();
        for t in &self.tables {
            for a in &t.ext {
                store.push(
                    t.name(),
                    ExtColumn {
                        name: a.name.clone(),
                        ty: a.ty,
                        default: a.default.clone(),
                        cells: t.rows.iter().map(|r| r.cells[a.column].clone()).collect(),
                    },
                );
            }
        }
        store
    }

    /// Index of the table called `name`, if any.
    pub fn table_index(&self, name: &[u8]) -> Option<usize> {
        self.tables.iter().position(|t| t.name().as_bytes() == name)
    }

    /// The data digest: SHA-256 over, per table by name: name, TAB,
    /// table digest, LF.
    pub fn digest(&self) -> String {
        let mut order: Vec<&PatchTable> = self.tables.iter().collect();
        order.sort_by(|a, b| a.name().cmp(b.name()));
        let mut h = Sha256::new();
        for t in order {
            h.update(t.name());
            h.update(b"\t");
            h.update(t.digest());
            h.update(b"\n");
        }
        hex(&h.finalize())
    }

    /// `layer:line` for a location.
    pub fn loc_name(&self, loc: Loc) -> String {
        let layer = self.layers.get(loc.layer).map_or("?", String::as_str);
        format!("{layer}:{}", loc.line)
    }

    /// The report form of a writer: `base line N` or `layer:line`.
    pub fn writer_name(&self, table: usize, row: usize, w: Writer) -> String {
        match w.loc() {
            Some(l) => self.loc_name(l),
            None => match self.tables[table].rows[row].origin {
                Origin::Base(line) => format!("base line {line}"),
                Origin::Added(l) => self.loc_name(l),
            },
        }
    }
}

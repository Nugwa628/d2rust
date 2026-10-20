//! The special-room tables of maze levels, read from their machine-readable source
//! `tables/drlg/maze-specials.tsv`; the code keeps no copy of the rows. A special
//! row says which cell def to look for and which special def, DS1 file and fallback
//! direction to stamp over it (see `cells` for the stamp).
//!
//! [`Specials::parse`] is strict: an unknown header, a malformed number, a direction
//! outside 0..3 or a row index that does not continue its table is an error naming
//! the line.

use std::collections::BTreeMap;

/// `tables/drlg/maze-specials.tsv`.
pub const SPECIALS_TSV: &str = include_str!("../../../../../tables/drlg/maze-specials.tsv");

/// The TSV header, column for column.
const HEADER: [&str; 9] = [
    "kind",
    "row",
    "find_def",
    "replace_def",
    "file",
    "fallback_dir",
    "va",
    "find_name",
    "replace_name",
];

/// One row of a special table: find def F, special def S, file f,
/// fallback direction d.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialRow {
    /// Def of the cell to replace (F).
    pub find: u32,
    /// Def stamped over it (S).
    pub special: u32,
    /// DS1 file index for the stamped cell (f).
    pub file: i32,
    /// Direction (0..3) in which a new cell is attached when no cell with the find def
    /// can be replaced (d).
    pub dir: u8,
}

/// Every special table by `kind`, rows in index order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Specials {
    tables: BTreeMap<String, Vec<SpecialRow>>,
}

impl Specials {
    /// Parses the TSV text (strict).
    pub fn parse(tsv: &str) -> Result<Self, String> {
        let mut lines = tsv.lines().enumerate();
        let header: Vec<&str> = lines.next().ok_or("empty TSV")?.1.split('\t').collect();
        if header != HEADER {
            return Err(format!("line 1: header {header:?}, expected {HEADER:?}"));
        }
        let mut tables: BTreeMap<String, Vec<SpecialRow>> = BTreeMap::new();
        let mut last_kind = String::new();
        for (i, line) in lines {
            let n = i + 1;
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() != HEADER.len() {
                return Err(format!("line {n}: {} columns, expected 9", cols.len()));
            }
            let num = |c: usize| -> Result<i64, String> {
                cols[c]
                    .parse::<i64>()
                    .map_err(|_| format!("line {n}: {} {:?} is not a number", HEADER[c], cols[c]))
            };
            let kind = cols[0];
            if kind.is_empty() {
                return Err(format!("line {n}: empty kind"));
            }
            if kind != last_kind && tables.contains_key(kind) {
                return Err(format!("line {n}: kind {kind} is not contiguous"));
            }
            let rows = tables.entry(kind.to_string()).or_default();
            if num(1)? != rows.len() as i64 {
                return Err(format!(
                    "line {n}: {kind} row {}, expected {}",
                    cols[1],
                    rows.len()
                ));
            }
            let find = u32::try_from(num(2)?).map_err(|_| format!("line {n}: bad find_def"))?;
            let special =
                u32::try_from(num(3)?).map_err(|_| format!("line {n}: bad replace_def"))?;
            let file = i32::try_from(num(4)?).map_err(|_| format!("line {n}: bad file"))?;
            let dir = num(5)?;
            if !(0..4).contains(&dir) {
                return Err(format!("line {n}: fallback_dir {dir} outside 0..3"));
            }
            let va = cols[6].strip_prefix("0x").unwrap_or("");
            if u32::from_str_radix(va, 16).is_err() {
                return Err(format!("line {n}: va {:?} is not 0x-hex", cols[6]));
            }
            rows.push(SpecialRow {
                find,
                special,
                file,
                dir: dir as u8,
            });
            last_kind = kind.to_string();
        }
        Ok(Self { tables })
    }

    /// The tables of the shipped TSV.
    pub fn shipped() -> Self {
        Self::parse(SPECIALS_TSV)
            .expect("tables/drlg/maze-specials.tsv parses (test `specials_tsv_parses`)")
    }

    /// Row `row` of table `kind`.
    pub fn row(&self, kind: &str, row: usize) -> Option<SpecialRow> {
        self.tables.get(kind)?.get(row).copied()
    }

    /// The rows of table `kind`.
    pub fn table(&self, kind: &str) -> Option<&[SpecialRow]> {
        self.tables.get(kind).map(Vec::as_slice)
    }

    /// Number of rows over all tables.
    pub fn len(&self) -> usize {
        self.tables.values().map(Vec::len).sum()
    }

    /// Whether there are no tables.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Kind names in sorted order.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

//! Cross-reference validation for the loaded `.bin` tables: every lookup field
//! (field types 11, 13, 15, 19–21) must hold an index below its linker's key
//! count, or the miss value −1 at its width. Broken links are reported with
//! table, row and column.
//!
//! Input is a [`BinSet`] plus the compile-only lookup tables from
//! [`load_lookups`]; output is a [`LinkerSizes`] map and a [`LinkReport`]. Used by
//! the load checks and by the cross-check tests of this crate.

use std::collections::BTreeMap;
use std::fmt;

use d2_formats::mpq::ArchiveSet;

use crate::bin::{excel_path, read_excel, tc_count, BinSet, BinTable, LoadError};
use crate::compile::{RANGE_LINKER, TC_LINKER};
use crate::schema::{schema, FieldDef, FieldType, LinkerKind};

/// Keys in `@range`: `none`, `h2h`, `rng`, `both`, `loc`.
const RANGE_KEYS: usize = 5;

/// Key count `n` of each linker, by `fields.tsv` link name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkerSizes {
    map: BTreeMap<String, usize>,
}

impl LinkerSizes {
    /// Sizes from the own-key fields of `tables`: a code or
    /// add-always key adds one index per record (`items.code` sums
    /// weapons, armor, misc); a find-or-add name key holds its index, so
    /// its size is the largest stored index + 1. The `<table>_lookup`
    /// linkers take the size of the runtime table compiled from the same
    /// `.txt`; `@range` has 5 keys and `@treasureclass` the TC count
    /// (see `tc_count`). A table name seen twice counts once.
    pub fn from_tables<'a>(tables: impl IntoIterator<Item = &'a BinTable>) -> LinkerSizes {
        let mut sizes = LinkerSizes::default();
        let mut seen: BTreeMap<&str, &BinTable> = BTreeMap::new();
        for t in tables {
            if seen.contains_key(t.name.as_str()) {
                continue;
            }
            seen.insert(&t.name, t);
            let Some((key, linker)) = own_key(&t.name) else {
                continue;
            };
            match key.field_type.id() {
                17 | 18 => {
                    let n = t
                        .iter()
                        .map(|r| read(r, key) as usize + 1)
                        .max()
                        .unwrap_or(0);
                    let e = sizes.map.entry(linker.to_owned()).or_insert(0);
                    *e = (*e).max(n);
                }
                _ => *sizes.map.entry(linker.to_owned()).or_insert(0) += t.count,
            }
        }
        for def in &schema().tables {
            let Some(base) = def.name.strip_suffix("_lookup") else {
                continue;
            };
            let (Some((key, linker)), Some(runtime)) = (own_key(&def.name), seen.get(base)) else {
                continue;
            };
            if sizes.map.contains_key(linker) {
                continue;
            }
            let n = match key.field_type.linker_kind() {
                Some(LinkerKind::Code) => Some(runtime.count),
                _ => own_key(base)
                    .filter(|(k, _)| k.column == key.column)
                    .and_then(|(_, l)| sizes.get(l)),
            };
            if let Some(n) = n {
                sizes.map.insert(linker.to_owned(), n);
            }
        }
        sizes.map.insert(RANGE_LINKER.to_owned(), RANGE_KEYS);
        if let (Some(it), Some(tc)) = (seen.get("itemtypes"), seen.get("treasureclassex")) {
            sizes.map.insert(TC_LINKER.to_owned(), tc_count(it, tc));
        }
        sizes
    }

    /// The key count of `linker`, if known.
    pub fn get(&self, linker: &str) -> Option<usize> {
        self.map.get(linker).copied()
    }

    /// Sets the key count of `linker`.
    pub fn insert(&mut self, linker: &str, size: usize) {
        self.map.insert(linker.to_owned(), size);
    }

    /// Iterates `(linker, key count)` in linker-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize)> {
        self.map.iter().map(|(k, &v)| (k.as_str(), v))
    }
}

/// The own-key field of a table and the linker it fills.
fn own_key(table: &str) -> Option<(&'static FieldDef, &'static str)> {
    let def = schema().table(table)?;
    let key = def.fields.iter().find(|f| f.field_type.is_own_key())?;
    Some((key, key.link.linker()?))
}

/// Byte width of a key or lookup field.
fn width(field_type: FieldType) -> usize {
    field_type.width(0).unwrap_or(0) as usize
}

/// The little-endian value of a field.
fn read(record: &[u8], field: &FieldDef) -> u32 {
    let o = field.offset as usize;
    record[o..o + width(field.field_type)]
        .iter()
        .rev()
        .fold(0, |v, &b| (v << 8) | u32::from(b))
}

/// A lookup field whose stored index is outside its linker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLink {
    /// Table holding the bad cell.
    pub table: String,
    /// Record index of the cell.
    pub row: usize,
    /// Column name of the cell.
    pub column: String,
    /// Byte offset of the field inside the record.
    pub offset: u32,
    /// Name of the linker the field points into.
    pub linker: String,
    /// The stored value.
    pub value: u32,
    /// The linker's key count.
    pub size: usize,
}

impl fmt::Display for BrokenLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} row {} column `{}` (+{}): {} is not an index of {} ({} keys)",
            self.table, self.row, self.column, self.offset, self.value, self.linker, self.size
        )
    }
}

/// A lookup field (table, column, linker).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkField {
    /// Table holding the field.
    pub table: String,
    /// Column name of the field.
    pub column: String,
    /// Name of the linker the field points into.
    pub linker: String,
}

/// Result of [`validate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkReport {
    /// In table order, then row, then field-list order.
    pub broken: Vec<BrokenLink>,
    /// Cells holding a valid index.
    pub valid: usize,
    /// Cells holding the miss value −1 (legal).
    pub misses: usize,
    /// Lookup fields not checked because their linker's size is unknown.
    pub unchecked: Vec<LinkField>,
}

impl LinkReport {
    /// True when no link is broken.
    pub fn is_clean(&self) -> bool {
        self.broken.is_empty()
    }
}

/// Checks every lookup field of `tables` against `sizes`. Tables
/// without a field list are skipped.
pub fn validate<'a>(
    tables: impl IntoIterator<Item = &'a BinTable>,
    sizes: &LinkerSizes,
) -> LinkReport {
    let mut report = LinkReport::default();
    for t in tables {
        let Some(def) = schema().table(&t.name) else {
            continue;
        };
        let fields: Vec<(&FieldDef, &str, u32, usize)> = def
            .fields
            .iter()
            .filter(|f| f.field_type.is_lookup())
            .filter_map(|f| {
                let linker = f.link.linker()?;
                let Some(size) = sizes.get(linker) else {
                    report.unchecked.push(LinkField {
                        table: t.name.clone(),
                        column: f.name(),
                        linker: linker.to_owned(),
                    });
                    return None;
                };
                let miss = u32::MAX >> (32 - 8 * width(f.field_type));
                Some((f, linker, miss, size))
            })
            .collect();
        for (row, rec) in t.iter().enumerate() {
            for &(f, linker, miss, size) in &fields {
                let value = read(rec, f);
                if value == miss {
                    report.misses += 1;
                } else if (value as usize) < size {
                    report.valid += 1;
                } else {
                    report.broken.push(BrokenLink {
                        table: t.name.clone(),
                        row,
                        column: f.name(),
                        offset: f.offset,
                        linker: linker.to_owned(),
                        value,
                        size,
                    });
                }
            }
        }
    }
    report
}

/// The `.bin` by-products of the compile-only lookup tables that no runtime
/// table overwrites and that the live set does not already hold (`hitclass`).
/// They give the sizes of the compile-only linkers; a missing file is skipped,
/// which leaves the fields linking to it unchecked.
pub fn load_lookups(set: &ArchiveSet) -> Result<Vec<BinTable>, LoadError> {
    let s = schema();
    let mut out = Vec::new();
    for def in s
        .called()
        .filter(|d| !d.is_runtime() && d.name != "hitclass")
    {
        if s.runtime().any(|r| r.bin_name == def.bin_name) {
            continue;
        }
        if let Some((source, bytes)) = read_excel(set, &def.bin_name)? {
            out.push(BinTable::parse(
                &def.name,
                &source,
                &excel_path(&def.bin_name),
                &bytes,
                def.record_size,
            )?);
        }
    }
    Ok(out)
}

/// Validates the live set: sizes from its tables, `hitclass` and the
/// `lookups` by-products; checks its 73 tables and `hitclass`.
pub fn validate_set(data: &BinSet, lookups: &[BinTable]) -> (LinkerSizes, LinkReport) {
    let all = || {
        data.tables
            .iter()
            .chain(std::iter::once(&data.hitclass))
            .chain(lookups)
    };
    let sizes = LinkerSizes::from_tables(all());
    let report = validate(
        data.tables.iter().chain(std::iter::once(&data.hitclass)),
        &sizes,
    );
    (sizes, report)
}

#[cfg(test)]
mod tests;

//! Acceptance check of the text compiler: load and validate every `.bin` in the
//! player's archives, compile every table's highest-priority `.txt` (patch, then
//! expansion, then base archive, so cross-archive sources come out of the archive
//! set by themselves; `levels.txt` for `leveldefs`), and compare count and every
//! record byte, plus the four code buffers.
//!
//! [`run`] does the whole job from an [`ArchiveSet`]; [`compare_sets`] compares an
//! already loaded and compiled pair. The result is a [`CrossCheck`] report read
//! by the ignored acceptance test below and by the mutation-kill tests.

use std::collections::BTreeMap;

use d2_formats::mpq::ArchiveSet;

use crate::bin::{self, read_excel, BinSet, BinTable, LoadError};
use crate::calc::{BufferReport, CalcDiag};
use crate::compile::{Compiled, DiagKind};
use crate::compile_set::{compile_all, CompileSetError, CompiledSet};
use crate::schema::{schema, CalcBuffer, TableDef};

/// Why a table is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// One of the 73 record tables the game loads at run time.
    Runtime,
    /// `hitclass`, read only by the client composite loader.
    ClientOnly,
    /// A compile-only lookup whose `.bin` by-product ships with the game.
    ByProduct,
}

/// One differing byte group, for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffExample {
    /// Record index.
    pub record: usize,
    /// Byte offset inside the record.
    pub offset: usize,
    /// Bytes in the shipped `.bin`.
    pub shipped: Vec<u8>,
    /// Bytes produced by the text compile.
    pub compiled: Vec<u8>,
}

/// Differences attributed to one field (or overlap group, or unwritten
/// byte).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldDiff {
    /// Number of records that differ.
    pub records: usize,
    /// Number of differing bytes.
    pub bytes: usize,
    /// A few sample differences.
    pub examples: Vec<DiffExample>,
}

/// Label for the one known, accepted difference: `monstats` record 707 `NameStr`
/// is 5382 in the shipped `.bin` and 11154 when compiled from the text.
pub const REASON_NAMESTR_707: &str = "monstats record 707 NameStr: shipped 5382, compiled 11154";

/// Comparison result for one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableReport {
    /// Table name.
    pub name: String,
    /// Why the table is compared.
    pub role: Role,
    /// Archive the compiled `.txt` came from.
    pub txt_source: String,
    /// Archive the shipped `.bin` came from, if one was found.
    pub bin_source: Option<String>,
    /// Archive `tables.tsv` names as the live source.
    pub expected_source: Option<String>,
    /// Record count of the compiled text.
    pub records_txt: usize,
    /// Record count of the shipped `.bin`, if one was found.
    pub records_bin: Option<usize>,
    /// Why no comparison was made, if none was.
    pub note: Option<String>,
    /// Explained differences: reason → (bytes, records).
    pub explained: BTreeMap<&'static str, (usize, usize)>,
    /// Unexplained differences by field label.
    pub mismatches: BTreeMap<String, FieldDiff>,
    /// Every differing record with its field labels (`~` prefix: explained).
    pub record_diffs: BTreeMap<usize, Vec<String>>,
}

impl TableReport {
    /// True when a shipped `.bin` was available to compare against.
    pub fn compared(&self) -> bool {
        self.records_bin.is_some()
    }

    /// True when both sides have the same record count.
    pub fn counts_equal(&self) -> bool {
        self.records_bin == Some(self.records_txt)
    }

    /// Byte-identical.
    pub fn identical(&self) -> bool {
        self.counts_equal() && self.explained.is_empty() && self.mismatches.is_empty()
    }

    /// Identical up to explained differences.
    pub fn matches(&self) -> bool {
        self.counts_equal() && self.mismatches.is_empty()
    }
}

/// Comparison result for one calc code buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferCheck {
    /// Which buffer.
    pub buffer: CalcBuffer,
    /// Archive the shipped buffer came from.
    pub source: String,
    /// Length in bytes of the shipped buffer.
    pub shipped: usize,
    /// Length in bytes of the compiled buffer.
    pub compiled: usize,
    /// Offset of the first differing byte, or `None` when identical.
    pub first_difference: Option<usize>,
}

impl BufferCheck {
    /// True when the two buffers are byte-identical.
    pub fn identical(&self) -> bool {
        self.first_difference.is_none()
    }
}

/// The full report.
#[derive(Debug, Clone)]
pub struct CrossCheck {
    /// One entry per compared table.
    pub tables: Vec<TableReport>,
    /// One entry per calc code buffer.
    pub buffers: Vec<BufferCheck>,
    /// Compiler diagnostics over all compiled lists, by kind.
    pub diagnostics: BTreeMap<DiagKind, usize>,
    /// Calc-expression diagnostics by buffer and kind.
    pub calc_diagnostics: BTreeMap<(CalcBuffer, CalcDiag), usize>,
    /// Calls of unknown table-specific callbacks, by callback name.
    pub unspecified_callbacks: BTreeMap<String, usize>,
    /// Validation of the shipped code buffers.
    pub code_reports: BTreeMap<CalcBuffer, BufferReport>,
}

impl CrossCheck {
    /// Runtime tables (73) that match (identical or explained).
    pub fn runtime_matching(&self) -> usize {
        self.tables
            .iter()
            .filter(|t| t.role == Role::Runtime && t.matches())
            .count()
    }
}

/// Why [`run`] or [`compare_sets`] could not finish.
#[derive(Debug, thiserror::Error)]
pub enum CrossCheckError {
    /// Loading the shipped `.bin` set failed.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Compiling the text set failed.
    #[error(transparent)]
    Compile(#[from] CompileSetError),
}

/// Loads the live set, compiles the text set and compares them.
pub fn run(set: &ArchiveSet) -> Result<CrossCheck, CrossCheckError> {
    let data = bin::load(set, bin::DEFAULT_LANGUAGE)?;
    let mut reader = |file: &str| read_excel(set, file).map_err(|e| e.to_string());
    let compiled = compile_all(&mut reader, &data.strings)?;
    Ok(compare_sets(set, &data, &compiled)?)
}

/// Compares a compiled text set with a loaded `.bin` set.
pub fn compare_sets(
    set: &ArchiveSet,
    data: &BinSet,
    compiled: &CompiledSet,
) -> Result<CrossCheck, LoadError> {
    let mut tables = Vec::new();
    let mut diagnostics = BTreeMap::new();
    for ct in &compiled.tables {
        for d in &ct.compiled.diagnostics {
            *diagnostics.entry(d.kind).or_default() += 1;
        }
        let def = schema()
            .table(&ct.name)
            .expect("compiled tables come from the schema");
        let (role, bin) = if def.is_runtime() {
            (Role::Runtime, data.table(&def.name).cloned())
        } else if def.name == "hitclass" {
            (Role::ClientOnly, Some(data.hitclass.clone()))
        } else if def.name.ends_with("_lookup") {
            continue; // its .bin is the runtime table's (the runtime table's own)
        } else {
            (Role::ByProduct, None)
        };
        let mut report = TableReport {
            name: def.name.clone(),
            role,
            txt_source: ct.txt_source.clone(),
            bin_source: None,
            expected_source: def.live_source.as_ref().map(|(a, _)| format!("{a}.mpq")),
            records_txt: ct.compiled.count,
            records_bin: None,
            note: None,
            explained: BTreeMap::new(),
            mismatches: BTreeMap::new(),
            record_diffs: BTreeMap::new(),
        };
        let bin = match bin {
            Some(b) => Some(b),
            None => match read_excel(set, &def.bin_name)? {
                None => {
                    report.note = Some("no .bin by-product in any archive".into());
                    None
                }
                Some((source, bytes)) => {
                    match BinTable::parse(
                        &def.name,
                        &source,
                        &def.bin_name,
                        &bytes,
                        def.record_size,
                    ) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            report.note = Some(format!("by-product not comparable: {e}"));
                            None
                        }
                    }
                }
            },
        };
        if let Some(bin) = bin {
            report.bin_source = Some(bin.source.clone());
            report.records_bin = Some(bin.count);
            if bin.count == ct.compiled.count {
                compare_records(def, &ct.compiled, &bin, &mut report);
            }
        }
        tables.push(report);
    }

    let buffers = CalcBuffer::ALL
        .iter()
        .map(|&b| {
            let shipped = &data.code[&b];
            let ours = &compiled.buffers[&b];
            let first_difference = (0..shipped.bytes.len().max(ours.len()))
                .find(|&i| shipped.bytes.get(i) != ours.get(i));
            BufferCheck {
                buffer: b,
                source: shipped.source.clone(),
                shipped: shipped.bytes.len(),
                compiled: ours.len(),
                first_difference,
            }
        })
        .collect();

    Ok(CrossCheck {
        tables,
        buffers,
        diagnostics,
        calc_diagnostics: compiled.calc_diagnostics.clone(),
        unspecified_callbacks: compiled.unspecified_callbacks.clone(),
        code_reports: data.code_reports.clone(),
    })
}

/// Field indices covering each record byte (empty footprints excluded).
fn coverage(def: &TableDef) -> Vec<Vec<usize>> {
    let mut cover = vec![Vec::new(); def.record_size];
    for (i, f) in def.fields.iter().enumerate() {
        for o in f.footprint() {
            if let Some(c) = cover.get_mut(o) {
                c.push(i);
            }
        }
    }
    cover
}

fn compare_records(def: &TableDef, compiled: &Compiled, bin: &BinTable, report: &mut TableReport) {
    let cover = coverage(def);
    let namestr = (def.name == "monstats")
        .then(|| def.field("NameStr").map(|f| f.offset as usize))
        .flatten();
    for r in 0..bin.count {
        let shipped = bin.record(r);
        let ours = compiled.record(r);
        if shipped == ours {
            continue;
        }
        let mut seen_explained: Vec<&'static str> = Vec::new();
        let mut seen_fields: Vec<String> = Vec::new();
        for o in (0..def.record_size).filter(|&o| shipped[o] != ours[o]) {
            let explained =
                if let Some(ns) = namestr.filter(|&ns| r == 707 && (ns..ns + 2).contains(&o)) {
                    (u16::from_le_bytes([shipped[ns], shipped[ns + 1]]) == 5382
                        && u16::from_le_bytes([ours[ns], ours[ns + 1]]) == 11154)
                        .then_some(REASON_NAMESTR_707)
                } else {
                    None
                };
            if let Some(reason) = explained {
                report
                    .record_diffs
                    .entry(r)
                    .or_default()
                    .push(format!("~{reason}"));
                let e = report.explained.entry(reason).or_default();
                e.0 += 1;
                if !seen_explained.contains(&reason) {
                    seen_explained.push(reason);
                    e.1 += 1;
                }
                continue;
            }
            let (label, range) = match cover[o].as_slice() {
                [] => (format!("unwritten byte +{o}"), o..o + 1),
                fields => (
                    fields
                        .iter()
                        .map(|&i| def.fields[i].name())
                        .collect::<Vec<_>>()
                        .join("/"),
                    def.fields[fields[0]].footprint(),
                ),
            };
            let rd = report.record_diffs.entry(r).or_default();
            if !rd.contains(&label) {
                rd.push(label.clone());
            }
            let d = report.mismatches.entry(label.clone()).or_default();
            d.bytes += 1;
            if !seen_fields.contains(&label) {
                seen_fields.push(label);
                d.records += 1;
                if d.examples.len() < 3 {
                    d.examples.push(DiffExample {
                        record: r,
                        offset: range.start,
                        shipped: shipped[range.clone()].to_vec(),
                        compiled: ours[range].to_vec(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The acceptance test: all 73 tables and the 4 code buffers match, up to
    /// the one difference named by `REASON_NAMESTR_707`.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn compiled_text_reproduces_live_bins() {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let report = run(&set).expect("cross-check runs");
        let runtime: Vec<&TableReport> = report
            .tables
            .iter()
            .filter(|t| t.role == Role::Runtime)
            .collect();
        assert_eq!(runtime.len(), 73);
        for t in &runtime {
            assert!(t.matches(), "{}: {:?}", t.name, t.mismatches);
            // The one explained difference left: monstats 707 NameStr.
            assert!(t.identical() || t.name == "monstats", "{}", t.name);
        }
        assert!(report.unspecified_callbacks.is_empty());
        for b in &report.buffers {
            assert!(b.identical(), "{:?}", b);
        }
    }
}

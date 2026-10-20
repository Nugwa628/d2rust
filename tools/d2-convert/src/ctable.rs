//! C-TABLE step 2 for the converter: compile the native excel set against
//! the install's live `.bin` tables and write `_bin-overrides.toml`.
//!
//! The overrides file has no source file. Its `files.tsv` row has kind
//! `excel`, archive `-` and no source SHA-256. Resume rule: the row is
//! never kept from an earlier run; it is rederived after the per-file loop
//! of every finished run, because it depends on all the other excel files.

use std::fs;
use std::path::Path;

use d2_data::bin::{self, BinSet};
use d2_formats::mpq::ArchiveSet;
use d2_native::manifest::{FileRow, NativeFile, Status};
use d2_native::tables::{recheck_c_table, run_c_table, TableCheck, OVERRIDES_PATH};

use crate::fsutil::{atomic_write, join, sha256_hex};

/// Archive column of the overrides row.
pub const DERIVED: &str = "-";

/// The tables C-TABLE step 2 checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableSummary {
    pub identical: Vec<String>,
    /// (table, first difference).
    pub failed: Vec<(String, String)>,
    pub overrides: usize,
}

impl TableSummary {
    pub fn checked(&self) -> usize {
        self.identical.len() + self.failed.len()
    }

    /// `73/73 tables identical to the live .bin (1 override)`.
    pub fn line(&self) -> String {
        format!(
            "{}/{} tables identical to the live .bin ({} override{})",
            self.identical.len(),
            self.checked(),
            self.overrides,
            if self.overrides == 1 { "" } else { "s" }
        )
    }

    fn from_check(c: TableCheck, overrides: usize) -> TableSummary {
        TableSummary {
            identical: c.identical,
            failed: c.failures.into_iter().map(|e| (e.file, e.detail)).collect(),
            overrides,
        }
    }
}

/// Whether the install has live `.bin` tables at all. A hand-made install
/// without them (the converter's own synthetic tests) skips step 2; a real
/// one never does.
pub fn applicable(set: &ArchiveSet) -> bool {
    d2_data::schema::schema()
        .runtime()
        .any(|d| set.contains(&bin::excel_path(&d.bin_name)))
}

fn live(set: &ArchiveSet, language: &str) -> Result<BinSet, String> {
    bin::load(set, &language.to_ascii_lowercase()).map_err(|e| format!("live .bin set: {e}"))
}

/// A reader over `<base>/data/global/excel/`.
fn native_reader(
    base: &Path,
) -> impl FnMut(&str) -> Result<Option<(String, Vec<u8>)>, String> + '_ {
    move |file| {
        let rel = format!("data/global/excel/{}", file.to_ascii_lowercase());
        match fs::read(join(base, &rel)) {
            Ok(b) => Ok(Some(("native".to_owned(), b))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{rel}: {e}")),
        }
    }
}

/// Runs step 2 after the per-file loop: writes the overrides file into
/// `base/` and returns its row and the summary. A failure of the whole
/// step comes back as a failed row, never as an error.
pub fn run(
    set: &ArchiveSet,
    language: &str,
    base: &Path,
) -> (FileRow, TableSummary, Vec<(String, String)>) {
    let mut row = FileRow {
        path: OVERRIDES_PATH.to_owned(),
        kind: "excel".into(),
        archive: DERIVED.into(),
        source_sha256: String::new(),
        native: Vec::new(),
        status: Status::Ok,
    };
    let fail = |mut row: FileRow, summary: TableSummary, detail: String| {
        row.status = Status::Failed("C-TABLE".into());
        let mut f = vec![(OVERRIDES_PATH.to_owned(), detail)];
        f.extend(
            summary
                .failed
                .iter()
                .map(|(t, d)| (format!("data/global/excel/{t}.bin"), format!("{t}: {d}"))),
        );
        (row, summary, f)
    };
    let result = live(set, language).and_then(|live| {
        let mut read = native_reader(base);
        run_c_table(&live, &mut read).map_err(|e| e.to_string())
    });
    let run = match result {
        Ok(r) => r,
        Err(e) => return fail(row, TableSummary::default(), e),
    };
    let text = run.overrides.to_toml();
    let n = run.overrides.entries.len();
    let summary = TableSummary::from_check(run.check, n);
    if !summary.failed.is_empty() {
        let d = format!(
            "{} of {} tables differ",
            summary.failed.len(),
            summary.checked()
        );
        return fail(row, summary, d);
    }
    let bytes = text.into_bytes();
    if let Err(e) = atomic_write(&join(base, OVERRIDES_PATH), &bytes) {
        return fail(row, summary, format!("write: {e}"));
    }
    row.native.push(NativeFile {
        name: OVERRIDES_PATH.to_owned(),
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    });
    (row, summary, Vec::new())
}

/// `verify --deep`: the same comparison against the files in `base/`.
/// Problems are lines naming the file.
pub fn recheck(set: &ArchiveSet, language: &str, base: &Path) -> (TableSummary, Vec<String>) {
    let text = match fs::read_to_string(join(base, OVERRIDES_PATH)) {
        Ok(t) => t,
        Err(e) => {
            return (
                TableSummary::default(),
                vec![format!("{OVERRIDES_PATH}: {e}")],
            )
        }
    };
    let result = live(set, language).and_then(|live| {
        let mut read = native_reader(base);
        let n = d2_native::tables::BinOverrides::from_toml(OVERRIDES_PATH, &text)
            .map(|o| o.entries.len())
            .map_err(|e| e.to_string())?;
        recheck_c_table(&live, &mut read, &text)
            .map(|c| (c, n))
            .map_err(|e| e.to_string())
    });
    match result {
        Err(e) => (
            TableSummary::default(),
            vec![format!("{OVERRIDES_PATH}: C-TABLE: {e}")],
        ),
        Ok((check, n)) => {
            let summary = TableSummary::from_check(check, n);
            let problems = summary
                .failed
                .iter()
                .map(|(t, d)| format!("{t}.bin: C-TABLE: {d}"))
                .collect();
            (summary, problems)
        }
    }
}

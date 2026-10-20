//! `report.txt`: human-readable, not parsed. The only place a date or a
//! run time may appear.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use d2_native::manifest::Manifest;

use crate::fsutil::utc_string;

pub struct Report<'a> {
    pub started_at: u64,
    pub install: String,
    pub manifest: &'a Manifest,
    /// (path, check, first difference).
    pub failures: &'a [(String, String, String)],
    /// (path, line) from the kinds (DT1 fallback tiles, `tbl` rebuilds).
    pub notes: &'a [(String, String)],
    pub unconverted: &'a BTreeMap<String, u64>,
    /// Paths skipped because the game never loads them.
    pub not_loaded: &'a [String],
    pub rejected_names: usize,
    pub kind_time: &'a BTreeMap<String, Duration>,
    pub wall: Duration,
    /// C-TABLE step 2.
    pub tables: Option<&'a crate::ctable::TableSummary>,
}

pub fn render(r: &Report) -> String {
    let m = r.manifest;
    let mut s = String::new();
    let _ = writeln!(s, "d2-convert report");
    let _ = writeln!(s, "run date:  {}", utc_string(r.started_at));
    let _ = writeln!(s, "install:   {}", r.install);
    let _ = writeln!(
        s,
        "converter: {} ({})",
        m.converter_version, m.converter_commit
    );
    let _ = writeln!(s, "complete:  {}", m.complete);
    let _ = writeln!(s, "\nper kind (converted / failed / skipped, work seconds)");
    for (k, c) in &m.counts {
        let t = r.kind_time.get(k).map_or(0.0, Duration::as_secs_f64);
        let _ = writeln!(
            s,
            "  {k:<10} {:>7} {:>6} {:>6}   {t:.1}",
            c.converted, c.failed, c.skipped
        );
    }
    let _ = writeln!(s, "total wall time: {:.1}s", r.wall.as_secs_f64());
    match r.tables {
        Some(t) => {
            let _ = writeln!(s, "\nC-TABLE step 2: {}", t.line());
            for n in &t.identical {
                let _ = writeln!(s, "  {n}: identical");
            }
            for (n, d) in &t.failed {
                let _ = writeln!(s, "  {n}: FAILED {d}");
            }
        }
        None => {
            let _ = writeln!(
                s,
                "\nC-TABLE step 2: not run (the install has no live .bin tables)"
            );
        }
    }
    let _ = writeln!(s, "\nfailures: {}", r.failures.len());
    for (path, check, detail) in r.failures {
        let _ = writeln!(s, "  {path}: {check}: {detail}");
    }
    let _ = writeln!(s, "\nnotes: {}", r.notes.len());
    for (path, line) in r.notes {
        let _ = writeln!(s, "  {path}: {line}");
    }
    let _ = writeln!(
        s,
        "\nnever loaded by the game (skipped): {}",
        r.not_loaded.len()
    );
    for p in r.not_loaded {
        let why = crate::kinds::never_loaded(p).unwrap_or("");
        let _ = writeln!(s, "  {p}: {why}");
    }
    let _ = writeln!(s, "\nunnamed blocks per archive (not converted)");
    for (a, n) in &m.unnamed_blocks {
        let _ = writeln!(s, "  {a}: {n}");
    }
    let _ = writeln!(
        s,
        "\nnames that cannot be native paths: {}",
        r.rejected_names
    );
    let _ = writeln!(s, "\nunconverted extensions (files, no reader)");
    for (e, n) in r.unconverted {
        match crate::kinds::SKIPPED_EXTENSIONS
            .iter()
            .find(|(x, _)| x == e)
        {
            Some((_, why)) => {
                let _ = writeln!(s, "  .{e}: {n} (skipped: {why})");
            }
            None => {
                let _ = writeln!(s, "  .{e}: {n}");
            }
        }
    }
    s
}

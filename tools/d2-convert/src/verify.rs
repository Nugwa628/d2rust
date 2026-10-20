//! `d2-convert verify`: re-checks an existing native root.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use d2_formats::mpq::ArchiveSet;
use d2_native::manifest::{parse_files_tsv, Manifest, Status};
use d2_native::tables::OVERRIDES_PATH;

use crate::ctable::{self, TableSummary};
use crate::fsutil::{join, sha256_file, sha256_hex, walk_files};
use crate::kind::{Kind, NativeData};
use crate::names::archive_name;

#[derive(Debug, Clone)]
pub struct VerifyOptions {
    pub out: PathBuf,
    /// `--deep`: re-run the table checks against this install.
    pub deep_install: Option<PathBuf>,
    pub progress: bool,
}

/// What `verify` found.
#[derive(Debug, Default)]
pub struct VerifyOutcome {
    /// One line per problem, naming the file. Empty = pass (exit 0).
    pub problems: Vec<String>,
    /// C-TABLE step 2 re-run by `--deep` (`None`: not deep, or the install
    /// has no live `.bin` tables).
    pub tables: Option<TableSummary>,
}

/// Every problem found, one line each, naming the file. Empty = pass
/// (exit 0).
pub fn verify(opts: &VerifyOptions, kinds: &[Box<dyn Kind>]) -> Vec<String> {
    verify_full(opts, kinds).problems
}

/// [`verify`] plus the tables `--deep` checked.
pub fn verify_full(opts: &VerifyOptions, kinds: &[Box<dyn Kind>]) -> VerifyOutcome {
    let mut tables = None;
    let problems = run(opts, kinds, &mut tables);
    VerifyOutcome { problems, tables }
}

fn run(
    opts: &VerifyOptions,
    kinds: &[Box<dyn Kind>],
    tables: &mut Option<TableSummary>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let out = &opts.out;
    let manifest = match fs::read_to_string(out.join("manifest.toml"))
        .map_err(|e| format!("manifest.toml: {e}"))
        .and_then(|t| Manifest::from_toml(&t).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => return vec![e],
    };
    if !manifest.complete {
        problems.push("manifest.toml: complete = false".into());
    }
    let tsv_bytes = match fs::read(out.join("files.tsv")) {
        Ok(b) => b,
        Err(e) => {
            problems.push(format!("files.tsv: {e}"));
            return problems;
        }
    };
    if sha256_hex(&tsv_bytes) != manifest.files_sha256 {
        problems.push("files.tsv: SHA-256 differs from manifest.toml files_sha256".into());
    }
    let rows = match parse_files_tsv(&String::from_utf8_lossy(&tsv_bytes), false) {
        Ok(r) => r,
        Err(e) => {
            problems.push(e.to_string());
            return problems;
        }
    };
    let base = out.join("base");
    let mut named = BTreeSet::new();
    for r in &rows {
        if let Status::Failed(c) = &r.status {
            problems.push(format!("{}: recorded as failed:{c}", r.path));
        }
        match manifest.kinds.get(&r.kind) {
            None => problems.push(format!(
                "{}: kind `{}` is not in manifest kinds",
                r.path, r.kind
            )),
            Some(v) => {
                if let Some(k) = kinds.iter().find(|k| k.name() == r.kind) {
                    if k.native_version() != *v {
                        problems.push(format!(
                            "{}: kind `{}` is native_version {v}, this build writes {}",
                            r.path,
                            r.kind,
                            k.native_version()
                        ));
                    }
                }
            }
        }
        for n in &r.native {
            named.insert(n.name.clone());
            match sha256_file(&join(&base, &n.name)) {
                Err(e) => problems.push(format!("{}: {e}", n.name)),
                Ok((sha, size)) => {
                    if size != n.size {
                        problems.push(format!("{}: size {size}, files.tsv has {}", n.name, n.size));
                    } else if sha != n.sha256 {
                        problems.push(format!("{}: SHA-256 differs from files.tsv", n.name));
                    }
                }
            }
        }
    }
    match walk_files(&base) {
        Ok(files) => {
            for f in files {
                if !named.contains(&f) {
                    problems.push(format!("{f}: in base/ but in no files.tsv row"));
                }
            }
        }
        Err(e) => problems.push(format!("base/: {e}")),
    }

    if let Some(install) = &opts.deep_install {
        let set = match ArchiveSet::open_dir(install) {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("{}: {e}", install.display()));
                return problems;
            }
        };
        let applicable = ctable::applicable(&set);
        let has_row = rows.iter().any(|r| r.path == OVERRIDES_PATH);
        if applicable && !has_row {
            problems.push(format!(
                "{OVERRIDES_PATH}: no files.tsv row (C-TABLE step 2 never ran)"
            ));
        }
        if applicable && has_row {
            let (summary, p) = ctable::recheck(&set, &manifest.language, &base);
            problems.extend(p);
            *tables = Some(summary);
        }
        for (i, r) in rows.iter().enumerate() {
            if r.status != Status::Ok || r.path == OVERRIDES_PATH {
                continue;
            }
            if opts.progress && i % 1000 == 0 {
                eprintln!("[{i}/{}] {}", rows.len(), r.path);
            }
            let Some(kind) = kinds.iter().find(|k| k.name() == r.kind) else {
                problems.push(format!("{}: this build has no kind `{}`", r.path, r.kind));
                continue;
            };
            let src = match set.read(&archive_name(&r.path)) {
                Ok(b) => b,
                Err(e) => {
                    problems.push(format!("{}: source: {e}", r.path));
                    continue;
                }
            };
            if sha256_hex(&src) != r.source_sha256 {
                problems.push(format!(
                    "{}: source SHA-256 differs (install changed)",
                    r.path
                ));
                continue;
            }
            let mut native = Vec::new();
            for n in &r.native {
                match fs::read(join(&base, &n.name)) {
                    Ok(bytes) => native.push(NativeData {
                        name: n.name.clone(),
                        bytes,
                    }),
                    Err(e) => problems.push(format!("{}: {e}", n.name)),
                }
            }
            if native.len() != r.native.len() {
                continue;
            }
            let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                kind.check(&r.path, &src, &native)
            }));
            match checked {
                Ok(Ok(())) => {}
                Ok(Err(f)) => problems.push(format!("{}: {}: {}", r.path, f.check, f.detail)),
                Err(_) => problems.push(format!("{}: the native reader panicked", r.path)),
            }
        }
    }
    problems
}

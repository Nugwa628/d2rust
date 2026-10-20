//! The conversion run: name set, per-file decode → write → read back →
//! check, atomic commit into `base/`, resume, manifest and report.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use d2_formats::mpq::flags;
use d2_formats::mpq::{archive_file_name, ArchiveSet, MpqError};
use d2_native::manifest::{
    check_native_name, parse_files_tsv, write_files_tsv, ArchiveId, FileRow, KindCounts, Manifest,
    NativeFile, Status, FILES_HEADER,
};

use crate::ctable::{self, TableSummary};
use crate::fsutil::{atomic_write, join, sha256_file, sha256_hex, walk_files};
use crate::kind::{Failure, Kind, NativeData};
use crate::names::{self, archive_name};
use crate::report::{self, Report};

/// Archives a D2 LoD install must have; video archives are
/// optional.
pub const REQUIRED_ARCHIVES: [&str; 9] = [
    "d2data.mpq",
    "d2exp.mpq",
    "d2char.mpq",
    "d2sfx.mpq",
    "d2speech.mpq",
    "patch_d2.mpq",
    "d2music.mpq",
    "d2xmusic.mpq",
    "d2xtalk.mpq",
];

#[derive(Debug, Clone)]
pub struct Options {
    pub install: PathBuf,
    pub out: PathBuf,
    pub language: String,
    /// Redo every file (`--force`).
    pub force: bool,
    /// Worker threads; 0 = the machine's parallelism.
    pub threads: usize,
    pub converter_version: String,
    pub converter_commit: String,
    /// Print progress to stderr.
    pub progress: bool,
    /// Test hook: stop after this many files were committed, as if the run
    /// were killed. Writes no manifest.
    pub stop_after: Option<usize>,
}

impl Options {
    pub fn new(install: impl Into<PathBuf>, out: impl Into<PathBuf>) -> Options {
        Options {
            install: install.into(),
            out: out.into(),
            language: "ENG".into(),
            force: false,
            threads: 0,
            converter_version: env!("CARGO_PKG_VERSION").into(),
            converter_commit: option_env!("D2_CONVERT_COMMIT").unwrap_or("unknown").into(),
            progress: false,
            stop_after: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// Exit code 2: not a D2 LoD install.
    #[error("{0}")]
    NotAnInstall(String),
    #[error("{what}: {source}")]
    Io {
        what: String,
        source: std::io::Error,
    },
    #[error("archive: {0}")]
    Mpq(#[from] MpqError),
    #[error("{0}")]
    State(String),
}

fn io_err(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> ConvertError {
    let what = what.into();
    move |source| ConvertError::Io { what, source }
}

#[derive(Debug)]
pub struct RunSummary {
    pub manifest: Option<Manifest>,
    pub rows: Vec<FileRow>,
    /// (path, check, first difference).
    pub failures: Vec<(String, String, String)>,
    /// False when `stop_after` ended the run early.
    pub finished: bool,
    /// C-TABLE step 2 (`None`: the install has no live `.bin`, or the run
    /// did not finish).
    pub tables: Option<TableSummary>,
}

impl RunSummary {
    /// 0 complete, 1 failures.
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.failures.is_empty())
    }
}

struct Item {
    canon: String,
    kind: usize,
    archive: String,
}

enum Done {
    Reused(FileRow),
    Converted {
        row: FileRow,
        tmp: PathBuf,
        notes: Vec<String>,
    },
    Failed {
        row: FileRow,
        tmp: PathBuf,
        failure: Failure,
    },
}

struct Timed {
    done: Done,
    took: Duration,
}

fn failed_row(item: &Item, kind: &dyn Kind, sha: String, check: &str) -> FileRow {
    FileRow {
        path: item.canon.clone(),
        kind: kind.name().into(),
        archive: item.archive.clone(),
        source_sha256: sha,
        native: Vec::new(),
        status: Status::Failed(check.to_owned()),
    }
}

/// The per-file pipeline, run on a worker. Writes under
/// `work_tmp/<idx>/`, reads back from disk, checks.
fn process(
    set: &ArchiveSet,
    kinds: &[Box<dyn Kind>],
    item: &Item,
    idx: usize,
    out: &Path,
    old: Option<&FileRow>,
) -> Done {
    let kind = kinds[item.kind].as_ref();
    let tmp = out.join(".work").join("tmp").join(idx.to_string());
    let src = match set.read(&archive_name(&item.canon)) {
        Ok(b) => b,
        Err(e) => {
            return Done::Failed {
                row: failed_row(item, kind, String::new(), "read"),
                tmp,
                failure: Failure::new("read", e.to_string()),
            }
        }
    };
    let sha = sha256_hex(&src);
    // Resume: same source, ok, files present with the recorded size.
    if let Some(old) = old {
        let same = old.status == Status::Ok
            && old.kind == kind.name()
            && old.source_sha256 == sha
            && old.archive == item.archive
            && old.native.iter().all(|n| {
                fs::metadata(join(&out.join("base"), &n.name)).is_ok_and(|m| m.len() == n.size)
            });
        if same {
            return Done::Reused(old.clone());
        }
    }
    let fail = |check: &str, detail: String| Done::Failed {
        row: failed_row(item, kind, sha.clone(), check),
        tmp: tmp.clone(),
        failure: Failure::new(check, detail),
    };
    let _ = fs::remove_dir_all(&tmp);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        kind.write(&item.canon, &src)
    }));
    let written = match unwound {
        Ok(Ok(w)) => w,
        Ok(Err(f)) => return fail(&f.check, f.detail),
        Err(_) => return fail("decode", "the reader panicked".into()),
    };
    let mut native = Vec::new();
    let mut read_back = Vec::new();
    let mut seen = BTreeSet::new();
    for f in &written.files {
        if let Err(e) = check_native_name(&f.name) {
            return fail("write", e);
        }
        if !seen.insert(f.name.clone()) {
            return fail("write", format!("{} written twice", f.name));
        }
        let path = join(&tmp, &f.name);
        if let Err(e) = atomic_write(&path, &f.bytes) {
            return fail("write", format!("{}: {e}", f.name));
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => return fail("read-back", format!("{}: {e}", f.name)),
        };
        native.push(NativeFile {
            name: f.name.clone(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
        read_back.push(NativeData {
            name: f.name.clone(),
            bytes,
        });
    }
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        kind.check(&item.canon, &src, &read_back)
    }));
    match checked {
        Ok(Ok(())) => {}
        Ok(Err(f)) => return fail(&f.check, f.detail),
        Err(_) => return fail("read-back", "the native reader panicked".into()),
    }
    Done::Converted {
        row: FileRow {
            path: item.canon.clone(),
            kind: kind.name().into(),
            archive: item.archive.clone(),
            source_sha256: sha,
            native,
            status: Status::Ok,
        },
        tmp,
        notes: written.notes,
    }
}

fn append_row(tsv: &mut fs::File, row: &FileRow) -> std::io::Result<()> {
    let text = write_files_tsv(std::slice::from_ref(row));
    let line = text.strip_prefix(FILES_HEADER).unwrap_or(&text);
    let line = line.strip_prefix('\n').unwrap_or(line);
    tsv.write_all(line.as_bytes())?;
    tsv.flush()
}

fn kind_versions(kinds: &[Box<dyn Kind>]) -> BTreeMap<String, u32> {
    kinds
        .iter()
        .map(|k| (k.name().to_owned(), k.native_version()))
        .collect()
}

fn versions_toml(m: &BTreeMap<String, u32>) -> String {
    let mut s = String::from("[kinds]\n");
    for (k, v) in m {
        s.push_str(&format!("{k} = {v}\n"));
    }
    s
}

/// Kind versions the previous run wrote (finished: `manifest.toml`; killed:
/// `.work/run.toml`).
fn previous_versions(out: &Path) -> BTreeMap<String, u32> {
    if let Ok(text) = fs::read_to_string(out.join("manifest.toml")) {
        if let Ok(m) = Manifest::from_toml(&text) {
            return m.kinds;
        }
    }
    let mut map = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(out.join(".work").join("run.toml")) {
        if let Ok(t) = text.parse::<toml::Table>() {
            if let Some(k) = t.get("kinds").and_then(|k| k.as_table()) {
                for (n, v) in k {
                    if let Some(v) = v.as_integer().and_then(|v| u32::try_from(v).ok()) {
                        map.insert(n.clone(), v);
                    }
                }
            }
        }
    }
    map
}

/// Deletes files under `base/` that no row names (`base/` never holds a
/// file without its row).
pub fn remove_orphans(out: &Path, rows: &[FileRow]) -> std::io::Result<usize> {
    let base = out.join("base");
    let named: BTreeSet<&str> = rows
        .iter()
        .flat_map(|r| r.native.iter().map(|n| n.name.as_str()))
        .collect();
    let mut removed = 0;
    for rel in walk_files(&base)? {
        if !named.contains(rel.as_str()) {
            fs::remove_file(join(&base, &rel))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn say(opts: &Options, msg: &str) {
    if opts.progress {
        eprintln!("{msg}");
    }
}

/// Runs the conversion.
pub fn convert(opts: &Options, kinds: &[Box<dyn Kind>]) -> Result<RunSummary, ConvertError> {
    let started = Instant::now();
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    // 1. Open the install.
    if !opts.install.is_dir() {
        return Err(ConvertError::NotAnInstall(format!(
            "{} is not a folder",
            opts.install.display()
        )));
    }
    say(opts, &format!("opening {}", opts.install.display()));
    let set = ArchiveSet::open_dir(&opts.install)?;
    for need in REQUIRED_ARCHIVES {
        if !set.has_archive(need) {
            return Err(ConvertError::NotAnInstall(format!(
                "{} has no {need}: not a D2 Lord of Destruction install",
                opts.install.display()
            )));
        }
    }
    let mut archives = Vec::new();
    for a in set.archives() {
        let name = archive_file_name(a).to_ascii_lowercase();
        say(opts, &format!("hashing {name}"));
        let (sha256, size) = sha256_file(a.path()).map_err(io_err(&name))?;
        archives.push(ArchiveId { name, size, sha256 });
    }
    archives.sort_by(|a, b| a.name.cmp(&b.name));

    // 2. Names and winning copies.
    say(opts, "building the name set");
    let nameset = names::build(&set, kinds)?;
    let mut items: Vec<Item> = Vec::new();
    let mut counts: BTreeMap<String, KindCounts> = kinds
        .iter()
        .map(|k| (k.name().to_owned(), KindCounts::default()))
        .collect();
    let mut unconverted: BTreeMap<String, u64> = BTreeMap::new();
    let mut not_loaded: Vec<String> = Vec::new();
    for (canon, claimed) in &nameset.names {
        match claimed {
            None => {
                let ext = canon
                    .rsplit_once('.')
                    .map_or("(none)", |(_, e)| e)
                    .to_owned();
                if set.contains(&archive_name(canon)) {
                    *unconverted.entry(ext).or_default() += 1;
                }
            }
            Some(k) if crate::kinds::never_loaded(canon).is_some() => {
                // Never loaded by the game; skipped, listed in the report.
                if set.contains(&archive_name(canon)) {
                    not_loaded.push(canon.clone());
                }
                if let Some(c) = counts.get_mut(kinds[*k].name()) {
                    c.skipped += 1;
                }
            }
            Some(k) => match set.find(&archive_name(canon)) {
                Some(a) => items.push(Item {
                    canon: canon.clone(),
                    kind: *k,
                    archive: archive_file_name(a).to_ascii_lowercase(),
                }),
                None => {
                    if let Some(c) = counts.get_mut(kinds[*k].name()) {
                        c.skipped += 1;
                    }
                }
            },
        }
    }
    // Tables first, then path order.
    items.sort_by(|a, b| (kinds[a.kind].phase(), &a.canon).cmp(&(kinds[b.kind].phase(), &b.canon)));
    let mut unnamed_blocks = BTreeMap::new();
    for a in set.archives() {
        let mut reached = BTreeSet::new();
        for canon in nameset.names.keys() {
            if let Some(i) = a.find(&archive_name(canon)) {
                reached.insert(i);
            }
        }
        for special in ["(listfile)", "(attributes)", "(signature)"] {
            if let Some(i) = a.find(special) {
                reached.insert(i);
            }
        }
        let unnamed = a
            .block_table()
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                b.has(flags::EXISTS) && !b.has(flags::DELETE_MARKER) && !reached.contains(i)
            })
            .count() as u64;
        unnamed_blocks.insert(archive_file_name(a).to_ascii_lowercase(), unnamed);
    }

    // 3. Prepare the native root; read what a previous run left.
    let out = &opts.out;
    let base = out.join("base");
    let work = out.join(".work");
    fs::create_dir_all(&base).map_err(io_err("creating base/"))?;
    let versions = kind_versions(kinds);
    let prev_versions = previous_versions(out);
    // The root is incomplete from here on: the game must not start from it.
    let _ = fs::remove_file(out.join("manifest.toml"));
    let _ = fs::remove_dir_all(work.join("tmp"));
    let _ = fs::remove_dir_all(work.join("failed"));
    fs::create_dir_all(work.join("tmp")).map_err(io_err("creating .work/"))?;
    atomic_write(&work.join("run.toml"), versions_toml(&versions).as_bytes())
        .map_err(io_err("writing .work/run.toml"))?;
    let tsv_path = out.join("files.tsv");
    let mut old_rows: BTreeMap<String, FileRow> = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(&tsv_path) {
        let parsed = parse_files_tsv(&text, true)
            .map_err(|e| ConvertError::State(format!("{}: {e}", tsv_path.display())))?;
        for r in parsed {
            old_rows.insert(r.path.clone(), r); // later rows win
        }
    }
    let mut keep: BTreeMap<String, FileRow> = BTreeMap::new();
    for item in &items {
        let kname = kinds[item.kind].name();
        let ver_ok = prev_versions.get(kname) == versions.get(kname);
        if let Some(r) = old_rows.get(&item.canon) {
            if !opts.force && ver_ok && r.status == Status::Ok && r.kind == kname {
                keep.insert(item.canon.clone(), r.clone());
            }
        }
    }
    let kept_rows: Vec<FileRow> = keep.values().cloned().collect();
    remove_orphans(out, &kept_rows).map_err(io_err("removing orphan files"))?;
    // files.tsv restarts from the kept rows (atomically); new rows append.
    atomic_write(&tsv_path, write_files_tsv(&kept_rows).as_bytes())
        .map_err(io_err("writing files.tsv"))?;
    let mut tsv = fs::OpenOptions::new()
        .append(true)
        .open(&tsv_path)
        .map_err(io_err("opening files.tsv"))?;

    // 4. Work.
    let total = items.len();
    say(opts, &format!("{total} files to check or convert"));
    let threads = if opts.threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        opts.threads
    };
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<(usize, Timed)>();
    let mut rows: BTreeMap<String, FileRow> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut notes: Vec<(String, String)> = Vec::new();
    let mut per_kind_time: BTreeMap<String, Duration> = BTreeMap::new();
    let mut committed = 0usize;
    let mut finished = true;
    let mut commit_err: Option<ConvertError> = None;
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            let tx = tx.clone();
            let (items, keep, set, next, stop) = (&items, &keep, &set, &next, &stop);
            s.spawn(move || loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let t = Instant::now();
                let done = process(set, kinds, item, i, &opts.out, keep.get(&item.canon));
                if tx
                    .send((
                        i,
                        Timed {
                            done,
                            took: t.elapsed(),
                        },
                    ))
                    .is_err()
                {
                    break;
                }
            });
        }
        drop(tx);
        // Commit in item order: files and rows do not depend on
        // which worker finished first.
        let mut pending: BTreeMap<usize, Timed> = BTreeMap::new();
        let mut want = 0usize;
        let mut last_say = Instant::now();
        for (i, t) in rx.iter() {
            pending.insert(i, t);
            while let Some(t) = pending.remove(&want) {
                let item = &items[want];
                *per_kind_time
                    .entry(kinds[item.kind].name().to_owned())
                    .or_default() += t.took;
                let r = commit(&base, &work, item, t.done, &mut tsv);
                want += 1;
                match r {
                    Ok(Committed {
                        row,
                        failure,
                        notes: n,
                    }) => {
                        for line in n {
                            notes.push((item.canon.clone(), line));
                        }
                        if let Some(f) = failure {
                            failures.push((item.canon.clone(), f.check, f.detail));
                        }
                        rows.insert(row.path.clone(), row);
                    }
                    Err(e) => {
                        commit_err = Some(e);
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                committed += 1;
                if opts.stop_after.is_some_and(|n| committed >= n) {
                    finished = false;
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
                if last_say.elapsed() > Duration::from_secs(2) {
                    say(opts, &format!("[{want}/{total}] {}", item.canon));
                    last_say = Instant::now();
                }
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
        }
    });
    if let Some(e) = commit_err {
        return Err(e);
    }
    if !finished {
        return Ok(RunSummary {
            manifest: None,
            rows: rows.into_values().collect(),
            failures,
            finished,
            tables: None,
        });
    }

    // 4b. C-TABLE step 2: the native excel set against the live `.bin`s
    // Its row is never resumed; it is rederived every run.
    let mut tables = None;
    if ctable::applicable(&set) {
        say(opts, "C-TABLE: compiling the native excel set");
        let (row, summary, fails) = ctable::run(&set, &opts.language, &base);
        for (path, detail) in fails {
            failures.push((path, "C-TABLE".to_owned(), detail));
        }
        rows.insert(row.path.clone(), row);
        say(opts, &format!("C-TABLE: {}", summary.line()));
        tables = Some(summary);
    }

    // 5. Final files.tsv (sorted), orphan sweep, manifest, report.
    let all_rows: Vec<FileRow> = rows.into_values().collect();
    for r in &all_rows {
        let c = counts.entry(r.kind.clone()).or_default();
        match r.status {
            Status::Ok => c.converted += 1,
            Status::Failed(_) => c.failed += 1,
        }
    }
    let tsv_text = write_files_tsv(&all_rows);
    drop(tsv);
    atomic_write(&tsv_path, tsv_text.as_bytes()).map_err(io_err("writing files.tsv"))?;
    remove_orphans(out, &all_rows).map_err(io_err("removing orphan files"))?;
    let manifest = Manifest {
        converter_version: opts.converter_version.clone(),
        converter_commit: opts.converter_commit.clone(),
        kinds: versions,
        language: opts.language.clone(),
        lod: set.has_archive("d2exp.mpq"),
        archives,
        counts,
        unnamed_blocks,
        complete: failures.is_empty(),
        files_sha256: sha256_hex(tsv_text.as_bytes()),
    };
    atomic_write(&out.join("manifest.toml"), manifest.to_toml().as_bytes())
        .map_err(io_err("writing manifest.toml"))?;
    let _ = fs::remove_dir_all(work.join("tmp"));
    let _ = fs::remove_file(work.join("run.toml"));
    let rep = Report {
        started_at,
        install: opts.install.display().to_string(),
        manifest: &manifest,
        failures: &failures,
        notes: &notes,
        unconverted: &unconverted,
        not_loaded: &not_loaded,
        rejected_names: nameset.rejected.len(),
        kind_time: &per_kind_time,
        wall: started.elapsed(),
        tables: tables.as_ref(),
    };
    atomic_write(&out.join("report.txt"), report::render(&rep).as_bytes())
        .map_err(io_err("writing report.txt"))?;
    say(
        opts,
        &format!(
            "done: {} files, {} failed ({:.1}s)",
            all_rows.len(),
            failures.len(),
            started.elapsed().as_secs_f64()
        ),
    );
    Ok(RunSummary {
        manifest: Some(manifest),
        rows: all_rows,
        failures,
        finished,
        tables,
    })
}

struct Committed {
    row: FileRow,
    failure: Option<Failure>,
    notes: Vec<String>,
}

/// Moves a checked file's native files into `base/` by rename, then appends
/// its row; a failed file's files go to `.work/failed/`.
fn commit(
    base: &Path,
    work: &Path,
    item: &Item,
    done: Done,
    tsv: &mut fs::File,
) -> Result<Committed, ConvertError> {
    let io = |what: &str| {
        let what = format!("{what} {}", item.canon);
        move |source| ConvertError::Io { what, source }
    };
    match done {
        Done::Reused(row) => Ok(Committed {
            row,
            failure: None,
            notes: Vec::new(),
        }),
        Done::Converted { row, tmp, notes } => {
            for n in &row.native {
                let to = join(base, &n.name);
                if let Some(dir) = to.parent() {
                    fs::create_dir_all(dir).map_err(io("creating a folder for"))?;
                }
                fs::rename(join(&tmp, &n.name), &to).map_err(io("moving into base/:"))?;
            }
            append_row(tsv, &row).map_err(io("appending the row of"))?;
            let _ = fs::remove_dir_all(&tmp);
            Ok(Committed {
                row,
                failure: None,
                notes,
            })
        }
        Done::Failed { row, tmp, failure } => {
            let dest = join(&work.join("failed"), &item.canon);
            let _ = fs::remove_dir_all(&dest);
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir).map_err(io("creating a folder for"))?;
            }
            if tmp.is_dir() {
                let _ = fs::rename(&tmp, &dest);
            }
            // Whatever was written, the failure has a place to look.
            let note = format!("{}: {}\n", failure.check, failure.detail);
            fs::create_dir_all(&dest).map_err(io("creating a folder for"))?;
            fs::write(dest.join("failure.txt"), note).map_err(io("recording the failure of"))?;
            append_row(tsv, &row).map_err(io("appending the row of"))?;
            Ok(Committed {
                row,
                failure: Some(failure),
                notes: Vec::new(),
            })
        }
    }
}

//! The two index files at the top of a native root:
//!
//! - `manifest.toml`, the versioned identity of one conversion: which
//!   converter wrote it, the layout version of every kind, the language and
//!   game edition converted, the source archives, per-kind counts, and
//!   whether the run finished;
//! - `files.tsv`, one row per converted source file: its kind, the archive
//!   its winning copy came from, the source hash, the native files written
//!   for it (with sizes and hashes) and the outcome of its round-trip check.
//!
//! The converter writes both; the runtime native source reads them to
//! decide whether a root can be used at all.
//!
//! Output is deterministic, so two conversions of the same install are
//! byte-identical: keys in a fixed order, maps sorted, rows sorted by path
//! in byte order, `\n` line ends, no timestamps. Readers are strict: an
//! unknown `format_version` or a key of the wrong type is an error that
//! names it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// `format` key of every manifest.
pub const FORMAT: &str = "d2rust-native";
/// The `format` earlier builds wrote (legacy-name: still read, so packs
/// converted before the rename keep loading; never written).
pub const LEGACY_FORMAT: &str = "d2rs-native";
/// Layout version of the manifest and native root.
pub const FORMAT_VERSION: u32 = 1;
/// The oldest converter version whose `base/` this build accepts; raised
/// when a native format changes.
pub const MIN_CONVERTER_VERSION: &str = "0.0.0";
/// `files.tsv` header row.
pub const FILES_HEADER: &str = "path\tkind\tarchive\tsource_sha256\tnative\tstatus";

/// Why a manifest or `files.tsv` could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// `manifest.toml` is malformed.
    #[error("manifest.toml: {0}")]
    Manifest(String),
    /// A `files.tsv` line (1-based `line`) is malformed (`msg` says how).
    #[error("files.tsv line {line}: {msg}")]
    Files { line: usize, msg: String },
    /// The manifest is valid but the native root cannot be used (see
    /// [`Manifest::check_loadable`]).
    #[error("native root not usable: {0}")]
    Unusable(String),
}

/// One archive the converter opened, by identity only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveId {
    /// The archive's file name.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: String,
}

/// Per-kind counts (the `counts` table).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindCounts {
    /// Files converted and verified.
    pub converted: u64,
    /// Files whose conversion or check failed.
    pub failed: u64,
    /// Files left out.
    pub skipped: u64,
}

/// The `manifest.toml` of a native root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Version of the converter that wrote the root.
    pub converter_version: String,
    /// Source commit of that converter.
    pub converter_commit: String,
    /// `native_version` of each kind used.
    pub kinds: BTreeMap<String, u32>,
    /// Language of the converted string tables.
    pub language: String,
    /// Whether the expansion's data was converted.
    pub lod: bool,
    /// The source archives, identity only.
    pub archives: Vec<ArchiveId>,
    /// Counts per kind name.
    pub counts: BTreeMap<String, KindCounts>,
    /// Hash-table entries no known file name reached, per archive name.
    pub unnamed_blocks: BTreeMap<String, u64>,
    /// Whether the conversion ran to the end.
    pub complete: bool,
    /// Lowercase hex SHA-256 of `files.tsv`.
    pub files_sha256: String,
}

/// A TOML basic string: `"..."` with quotes, backslashes and control
/// characters escaped.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A TOML key: bare when it is made of letters, digits, `_` and `-` only,
/// quoted otherwise.
fn key(k: &str) -> String {
    let bare = !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        k.to_owned()
    } else {
        quote(k)
    }
}

fn malformed(msg: String) -> ManifestError {
    ManifestError::Manifest(msg)
}

/// Typed lookups in one table of the manifest. A missing key or a value of
/// the wrong type is a [`ManifestError::Manifest`] naming the key.
struct Keys<'a>(&'a toml::Table);

impl Keys<'_> {
    fn str(&self, k: &str) -> Result<String, ManifestError> {
        self.0
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| malformed(format!("`{k}` missing or not a string")))
    }

    fn count(&self, k: &str) -> Result<u64, ManifestError> {
        self.0
            .get(k)
            .and_then(|v| v.as_integer())
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| malformed(format!("`{k}` missing or not a non-negative integer")))
    }

    fn bool(&self, k: &str) -> Result<bool, ManifestError> {
        self.0
            .get(k)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| malformed(format!("`{k}` missing or not a boolean")))
    }

    fn table(&self, k: &str) -> Result<toml::Table, ManifestError> {
        self.0
            .get(k)
            .and_then(|v| v.as_table())
            .cloned()
            .ok_or_else(|| malformed(format!("`[{k}]` missing")))
    }
}

impl Manifest {
    /// The manifest as TOML text, keys in a fixed order.
    pub fn to_toml(&self) -> String {
        let mut s = String::new();
        let mut line = |text: String| {
            s.push_str(&text);
            s.push('\n');
        };
        line(format!("format = {}", quote(FORMAT)));
        line(format!("format_version = {FORMAT_VERSION}"));
        line(format!("complete = {}", self.complete));
        line(format!("files_sha256 = {}", quote(&self.files_sha256)));

        line("\n[converter]".into());
        line(format!("version = {}", quote(&self.converter_version)));
        line(format!("commit = {}", quote(&self.converter_commit)));

        line("\n[kinds]".into());
        for (k, v) in &self.kinds {
            line(format!("{} = {v}", key(k)));
        }

        line("\n[source]".into());
        line(format!("language = {}", quote(&self.language)));
        line(format!("lod = {}", self.lod));
        for a in &self.archives {
            line("\n[[source.archive]]".into());
            line(format!("name = {}", quote(&a.name)));
            line(format!("size = {}", a.size));
            line(format!("sha256 = {}", quote(&a.sha256)));
        }

        for (k, c) in &self.counts {
            line(format!("\n[counts.{}]", key(k)));
            line(format!("converted = {}", c.converted));
            line(format!("failed = {}", c.failed));
            line(format!("skipped = {}", c.skipped));
        }

        line("\n[unnamed_blocks]".into());
        for (k, v) in &self.unnamed_blocks {
            line(format!("{} = {v}", key(k)));
        }
        s
    }

    /// Parses a manifest. An unknown `format` or `format_version` is
    /// rejected before anything else is read.
    pub fn from_toml(text: &str) -> Result<Manifest, ManifestError> {
        let root: toml::Table = text.parse().map_err(|e| malformed(format!("{e}")))?;
        let top = Keys(&root);

        let format = top.str("format")?;
        if format != FORMAT && format != LEGACY_FORMAT {
            return Err(malformed(format!("format `{format}`, expected `{FORMAT}`")));
        }
        let version = top.count("format_version")?;
        if version != u64::from(FORMAT_VERSION) {
            return Err(malformed(format!(
                "format_version {version} is not known (this build reads {FORMAT_VERSION})"
            )));
        }

        let converter = top.table("converter")?;
        let mut kinds = BTreeMap::new();
        for (k, v) in top.table("kinds")? {
            let n = v
                .as_integer()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| malformed(format!("kinds.{k} is not a version number")))?;
            kinds.insert(k, n);
        }
        let source = top.table("source")?;
        let archives = parse_archives(&source)?;
        let mut counts = BTreeMap::new();
        for (k, v) in top.table("counts")? {
            let c = v
                .as_table()
                .ok_or_else(|| malformed(format!("counts.{k} is not a table")))?;
            let c = Keys(c);
            let row = KindCounts {
                converted: c.count("converted")?,
                failed: c.count("failed")?,
                skipped: c.count("skipped")?,
            };
            counts.insert(k, row);
        }
        let mut unnamed_blocks = BTreeMap::new();
        for (k, v) in top.table("unnamed_blocks")? {
            let n = v
                .as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| malformed(format!("unnamed_blocks.{k} is not a count")))?;
            unnamed_blocks.insert(k, n);
        }

        let (converter, source) = (Keys(&converter), Keys(&source));
        Ok(Manifest {
            converter_version: converter.str("version")?,
            converter_commit: converter.str("commit")?,
            kinds,
            language: source.str("language")?,
            lod: source.bool("lod")?,
            archives,
            counts,
            unnamed_blocks,
            complete: top.bool("complete")?,
            files_sha256: top.str("files_sha256")?,
        })
    }

    /// The conditions for using a native root: `complete = true`, every
    /// kind at the version this build reads (`known` maps each kind this
    /// build reads to its version), and a converter version at least
    /// [`MIN_CONVERTER_VERSION`]. The error names the first reason.
    pub fn check_loadable(&self, known: &BTreeMap<String, u32>) -> Result<(), ManifestError> {
        let unusable = |m: String| Err(ManifestError::Unusable(m));
        if !self.complete {
            return unusable("the conversion is not complete; run the converter".into());
        }
        for (k, v) in &self.kinds {
            match known.get(k) {
                Some(n) if n == v => {}
                Some(n) => {
                    return unusable(format!(
                        "kind `{k}` is native_version {v}, this build reads {n}"
                    ))
                }
                None => return unusable(format!("kind `{k}` is not known to this build")),
            }
        }
        if version_tuple(&self.converter_version) < version_tuple(MIN_CONVERTER_VERSION) {
            return unusable(format!(
                "converter {} is older than the minimum {MIN_CONVERTER_VERSION}",
                self.converter_version
            ));
        }
        Ok(())
    }
}

/// The `[[source.archive]]` entries (none when the key is absent).
fn parse_archives(source: &toml::Table) -> Result<Vec<ArchiveId>, ManifestError> {
    let Some(list) = source.get("archive") else {
        return Ok(Vec::new());
    };
    let list = list
        .as_array()
        .ok_or_else(|| malformed("source.archive is not an array".into()))?;
    list.iter()
        .map(|a| {
            let a = a
                .as_table()
                .ok_or_else(|| malformed("source.archive entry is not a table".into()))?;
            let a = Keys(a);
            Ok(ArchiveId {
                name: a.str("name")?,
                size: a.count("size")?,
                sha256: a.str("sha256")?,
            })
        })
        .collect()
}

/// The first three numbers of a version string, for ordering. `.`, `-` and
/// `+` all separate parts; a missing or non-numeric part counts as 0.
fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut parts = v
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u64>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next())
}

/// One native file of a source file, written in `files.tsv` as
/// `name:size:sha256`. `name` is relative to `base/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NativeFile {
    /// Path relative to `base/`.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

/// The `status` column: `ok`, or `failed:<check>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Converted and verified.
    Ok,
    /// The named check failed.
    Failed(String),
}

/// One `files.tsv` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    /// Canonical source path.
    pub path: String,
    /// Kind name (see `AssetKind::name`).
    pub kind: String,
    /// Name of the archive the winning copy came from.
    pub archive: String,
    /// Lowercase hex SHA-256 of the source file.
    pub source_sha256: String,
    /// The native files written for it.
    pub native: Vec<NativeFile>,
    /// Outcome.
    pub status: Status,
}

/// Whether `name` can sit in a `files.tsv` cell and in a `native` list
/// without breaking the row grammar: no tab, line end, `;`, `:`, `\` or
/// other control character, no empty, `.` or `..` part, and not absolute.
pub fn check_native_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty name".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '\t' | '\n' | '\r' | ';' | ':' | '\\') || c.is_control())
    {
        return Err(format!(
            "`{name}`: character {c:?} cannot be in a native name"
        ));
    }
    if name
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(format!("`{name}`: empty or dot path part"));
    }
    Ok(())
}

impl FileRow {
    /// The row as one `files.tsv` line, with its native files sorted and
    /// any tab or line end in a failed check's name turned into a space.
    fn to_line(&self) -> String {
        let mut native = self.native.clone();
        native.sort();
        let native = native
            .iter()
            .map(|n| format!("{}:{}:{}", n.name, n.size, n.sha256))
            .collect::<Vec<_>>()
            .join(";");
        let status = match &self.status {
            Status::Ok => "ok".to_owned(),
            Status::Failed(c) => format!("failed:{}", c.replace(['\t', '\n', '\r'], " ")),
        };
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            self.path, self.kind, self.archive, self.source_sha256, native, status
        )
    }

    /// Parses line `no` (1-based) of `files.tsv`.
    fn from_line(line: &str, no: usize) -> Result<FileRow, ManifestError> {
        let err = |msg: String| ManifestError::Files { line: no, msg };
        let cols: Vec<&str> = line.split('\t').collect();
        let [path, kind, archive, source_sha256, native, status] = cols[..] else {
            return Err(err(format!("{} columns, expected 6", cols.len())));
        };
        let native = if native.is_empty() {
            Vec::new()
        } else {
            native
                .split(';')
                .map(|item| parse_native_entry(item).map_err(err))
                .collect::<Result<_, _>>()?
        };
        let status = match status {
            "ok" => Status::Ok,
            s => match s.strip_prefix("failed:") {
                Some(c) => Status::Failed(c.to_owned()),
                None => return Err(err(format!("status `{s}` is not ok or failed:<check>"))),
            },
        };
        Ok(FileRow {
            path: path.to_owned(),
            kind: kind.to_owned(),
            archive: archive.to_owned(),
            source_sha256: source_sha256.to_owned(),
            native,
            status,
        })
    }
}

/// One `name:size:sha256` entry of the `native` column. The name is split
/// off at the last two colons.
fn parse_native_entry(item: &str) -> Result<NativeFile, String> {
    let mut parts = item.rsplitn(3, ':');
    let (Some(sha256), Some(size), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("native entry `{item}` is not name:size:sha256"));
    };
    Ok(NativeFile {
        name: name.to_owned(),
        size: size
            .parse()
            .map_err(|_| format!("native size `{size}` is not a number"))?,
        sha256: sha256.to_owned(),
    })
}

/// `files.tsv` text: the header, then the rows sorted by `path` in byte
/// order.
pub fn write_files_tsv(rows: &[FileRow]) -> String {
    let mut sorted: Vec<&FileRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut s = String::from(FILES_HEADER);
    s.push('\n');
    for r in sorted {
        s.push_str(&r.to_line());
    }
    s
}

/// Parses `files.tsv`. With `lenient_tail`, a last line without its `\n`
/// is dropped instead of being an error: the converter appends each row
/// only after that file's native files are in place, so a row torn by a
/// crash just means the file is converted again.
pub fn parse_files_tsv(text: &str, lenient_tail: bool) -> Result<Vec<FileRow>, ManifestError> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // `split` leaves an empty last element when the text ends in `\n`.
    let tail = lines.pop().unwrap_or("");
    if !tail.is_empty() && !lenient_tail {
        return Err(ManifestError::Files {
            line: lines.len() + 1,
            msg: "last line has no line end".into(),
        });
    }
    let mut numbered = lines.into_iter().enumerate();
    match numbered.next() {
        Some((_, header)) if header == FILES_HEADER => {}
        Some(_) => {
            return Err(ManifestError::Files {
                line: 1,
                msg: "header row is not the files.tsv header".into(),
            })
        }
        None if lenient_tail => return Ok(Vec::new()),
        None => {
            return Err(ManifestError::Files {
                line: 1,
                msg: "empty file".into(),
            })
        }
    }
    numbered
        .map(|(i, line)| FileRow::from_line(line, i + 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut kinds = BTreeMap::new();
        kinds.insert("dc6".to_owned(), 1);
        kinds.insert("excel".to_owned(), 1);
        let mut counts = BTreeMap::new();
        counts.insert(
            "dc6".to_owned(),
            KindCounts {
                converted: 3,
                failed: 1,
                skipped: 2,
            },
        );
        let mut unnamed = BTreeMap::new();
        unnamed.insert("d2data.mpq".to_owned(), 7);
        Manifest {
            converter_version: "0.1.2".into(),
            converter_commit: "abc123".into(),
            kinds,
            language: "ENG".into(),
            lod: true,
            archives: vec![ArchiveId {
                name: "d2data.mpq".into(),
                size: 99,
                sha256: "ab".repeat(32),
            }],
            counts,
            unnamed_blocks: unnamed,
            complete: false,
            files_sha256: "cd".repeat(32),
        }
    }

    #[test]
    fn manifest_round_trips_and_is_deterministic() {
        let m = sample();
        let text = m.to_toml();
        assert_eq!(text, m.to_toml());
        assert_eq!(Manifest::from_toml(&text).unwrap(), m);
        assert!(text.starts_with("format = \"d2rust-native\"\nformat_version = 1\n"));
        assert!(!text.contains("time") && !text.contains("date"));
    }

    #[test]
    fn unknown_format_version_is_rejected() {
        let text = sample()
            .to_toml()
            .replace("format_version = 1", "format_version = 2");
        let e = Manifest::from_toml(&text).unwrap_err();
        assert!(e.to_string().contains("format_version 2"), "{e}");
        let text = sample().to_toml().replace("d2rust-native", "other");
        assert!(Manifest::from_toml(&text).is_err());
    }

    #[test]
    fn a_pack_converted_before_the_rename_still_loads() {
        let m = sample();
        let text = m.to_toml().replace(FORMAT, LEGACY_FORMAT);
        assert_eq!(Manifest::from_toml(&text).unwrap(), m);
    }

    #[test]
    fn loadable_needs_complete_and_known_kinds() {
        let mut m = sample();
        let mut known: BTreeMap<String, u32> = m.kinds.clone();
        assert!(m
            .check_loadable(&known)
            .unwrap_err()
            .to_string()
            .contains("not complete"));
        m.complete = true;
        m.check_loadable(&known).unwrap();
        known.insert("dc6".into(), 2);
        assert!(m
            .check_loadable(&known)
            .unwrap_err()
            .to_string()
            .contains("dc6"));
        known.remove("dc6");
        assert!(m
            .check_loadable(&known)
            .unwrap_err()
            .to_string()
            .contains("not known"));
    }

    fn row(path: &str, status: Status) -> FileRow {
        FileRow {
            path: path.into(),
            kind: "dc6".into(),
            archive: "d2data.mpq".into(),
            source_sha256: "11".repeat(32),
            native: vec![
                NativeFile {
                    name: format!("{path}.toml"),
                    size: 5,
                    sha256: "22".repeat(32),
                },
                NativeFile {
                    name: format!("{path}.png"),
                    size: 9,
                    sha256: "33".repeat(32),
                },
            ],
            status,
        }
    }

    #[test]
    fn files_tsv_sorts_rows_and_native_lists() {
        let rows = vec![
            row("data/b.dc6", Status::Ok),
            row("data/a.dc6", Status::Failed("C-DC6".into())),
        ];
        let text = write_files_tsv(&rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], FILES_HEADER);
        assert!(lines[1].starts_with("data/a.dc6\t"));
        assert!(lines[1].ends_with("\tfailed:C-DC6"));
        // native entries sorted by name: `.png` before `.toml`
        assert!(lines[2].contains("data/b.dc6.png:9:"));
        assert!(lines[2].find(".png:").unwrap() < lines[2].find(".toml:").unwrap());
        let back = parse_files_tsv(&text, false).unwrap();
        assert_eq!(write_files_tsv(&back), text);
    }

    #[test]
    fn torn_last_row_is_dropped_only_when_lenient() {
        let text = write_files_tsv(&[row("a", Status::Ok), row("b", Status::Ok)]);
        let torn = &text[..text.len() - 10];
        assert!(parse_files_tsv(torn, false).is_err());
        let rows = parse_files_tsv(torn, true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "a");
    }

    #[test]
    fn bad_rows_name_their_line() {
        let text = format!("{FILES_HEADER}\nonly\ttwo\n");
        let e = parse_files_tsv(&text, false).unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
        let text = format!("{FILES_HEADER}\na\tk\tz\th\t\tmaybe\n");
        assert!(parse_files_tsv(&text, false).is_err());
    }

    #[test]
    fn native_names_cannot_break_the_row_grammar() {
        check_native_name("data/global/a.dc6.png").unwrap();
        for bad in ["a;b", "a:b", "a\tb", "a//b", "../x", "", "a\\b"] {
            assert!(check_native_name(bad).is_err(), "{bad}");
        }
    }
}

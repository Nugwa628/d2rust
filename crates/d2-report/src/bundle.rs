//! A bug report bundle in memory: the manifest and the bytes of each file.
//!
//! A [`Bundle`] is valid by construction: it is built file by file with
//! every limit checked on the way ([`Bundle::new`], [`Bundle::with_file`]), or
//! read from a folder or an upload and checked as a whole
//! ([`Bundle::read_dir`], [`Bundle::from_parts`]). The game writes one to
//! `bug-reports/<time>/`; the launcher and the report service read it back.
//!
//! On disk a bundle is a flat folder: `report.json` plus the fixed file names
//! of [`FileKind`]. Anything else in the folder (another file, a subfolder, a
//! link) makes the bundle invalid, so reading one never follows a path the
//! report chose.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{
    FileEntry, FileKind, Manifest, ReportError, StateDump, MANIFEST_NAME, MAX_LABEL_CHARS,
    MAX_LOG_LINES, MAX_MANIFEST_BYTES, MAX_TOTAL_BYTES,
};

/// What a new report says about itself, besides its files.
#[derive(Clone, Debug)]
pub struct Meta {
    pub game_version: String,
    pub launcher_version: Option<String>,
    pub os: String,
    pub time_unix: u64,
    pub note: String,
}

/// A checked bug report; see the module header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    manifest: Manifest,
    files: BTreeMap<FileKind, Vec<u8>>,
}

impl Bundle {
    /// A bundle with no files yet. Fails if a field breaks its limit.
    pub fn new(meta: Meta) -> Result<Bundle, ReportError> {
        let manifest = Manifest {
            game_version: meta.game_version,
            launcher_version: meta.launcher_version,
            os: meta.os,
            time_unix: meta.time_unix,
            note: meta.note,
            files: Vec::new(),
        };
        manifest.check_fields()?;
        Ok(Bundle {
            manifest,
            files: BTreeMap::new(),
        })
    }

    /// The bundle with `bytes` as its file of `kind` (replacing any earlier
    /// one). Fails, leaving nothing changed, if the file or the bundle would
    /// break a limit.
    pub fn with_file(mut self, kind: FileKind, bytes: Vec<u8>) -> Result<Bundle, ReportError> {
        check_content(kind, &bytes)?;
        self.files.insert(kind, bytes);
        self.sync_manifest();
        self.check_total()?;
        Ok(self)
    }

    /// The bundle with `note` as the player's note. Fails if the note breaks
    /// its limit.
    pub fn with_note(mut self, note: String) -> Result<Bundle, ReportError> {
        self.manifest.note = note;
        self.manifest.check_fields()?;
        self.check_total()?;
        Ok(self)
    }

    /// The bundle without its file of `kind` (the launcher lets the player
    /// leave the screenshot or the log out).
    pub fn without_file(mut self, kind: FileKind) -> Bundle {
        self.files.remove(&kind);
        self.sync_manifest();
        self
    }

    /// The manifest, as `report.json` will say it.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The bytes of the file of `kind`, if the bundle has one.
    pub fn file(&self, kind: FileKind) -> Option<&[u8]> {
        self.files.get(&kind).map(Vec::as_slice)
    }

    /// The bundle's size: `report.json` plus every file.
    pub fn total_bytes(&self) -> u64 {
        self.manifest.to_json().len() as u64
            + self.files.values().map(|f| f.len() as u64).sum::<u64>()
    }

    /// Checks a bundle that arrives as parts (an upload): `report.json`'s
    /// bytes and each other file as (name, bytes). Names are untrusted and
    /// must be exactly one of the fixed file names.
    pub fn from_parts(
        manifest: &[u8],
        parts: Vec<(String, Vec<u8>)>,
    ) -> Result<Bundle, ReportError> {
        let total = manifest.len() as u64 + parts.iter().map(|(_, b)| b.len() as u64).sum::<u64>();
        if total > MAX_TOTAL_BYTES {
            return Err(too_large("the bug report", total, MAX_TOTAL_BYTES));
        }
        let manifest = Manifest::parse(manifest)?;
        let mut files = BTreeMap::new();
        for (name, bytes) in parts {
            let kind = FileKind::from_file_name(&name)
                .ok_or_else(|| ReportError::UnknownFile(crate::clip(&name, MAX_LABEL_CHARS)))?;
            if files.insert(kind, bytes).is_some() {
                return Err(ReportError::Duplicate(kind.file_name()));
            }
        }
        for kind in FileKind::ALL {
            let name = kind.file_name();
            match (manifest.entry(kind), files.get(&kind)) {
                (None, None) => {}
                (Some(_), None) => return Err(ReportError::Missing(name)),
                (None, Some(_)) => return Err(ReportError::Unlisted(name)),
                (Some(e), Some(b)) => {
                    if e.size != b.len() as u64 {
                        return Err(ReportError::Mismatch {
                            file: name,
                            what: "size",
                        });
                    }
                    if e.sha256 != sha256(b) {
                        return Err(ReportError::Mismatch {
                            file: name,
                            what: "SHA-256",
                        });
                    }
                    check_content(kind, b)?;
                }
            }
        }
        Ok(Bundle { manifest, files })
    }

    /// Reads and checks the bundle in folder `dir`.
    pub fn read_dir(dir: &Path) -> Result<Bundle, ReportError> {
        let mut manifest = None;
        let mut parts = Vec::new();
        let mut total = 0u64;
        for item in std::fs::read_dir(dir)? {
            let item = item?;
            let raw = item.file_name();
            let Some(name) = raw.to_str() else {
                return Err(ReportError::UnknownFile(raw.to_string_lossy().into_owned()));
            };
            let unknown = || ReportError::UnknownFile(crate::clip(name, MAX_LABEL_CHARS));
            let max = if name == MANIFEST_NAME {
                MAX_MANIFEST_BYTES
            } else {
                FileKind::from_file_name(name)
                    .ok_or_else(unknown)?
                    .max_bytes()
            };
            // Links and folders are refused, not followed.
            let meta = std::fs::symlink_metadata(item.path())?;
            if !meta.file_type().is_file() {
                return Err(unknown());
            }
            if meta.len() > max {
                return Err(too_large(name, meta.len(), max));
            }
            total += meta.len();
            if total > MAX_TOTAL_BYTES {
                return Err(too_large("the bug report", total, MAX_TOTAL_BYTES));
            }
            let bytes = read_capped(&item.path(), max)?;
            if name == MANIFEST_NAME {
                manifest = Some(bytes);
            } else {
                parts.push((name.to_owned(), bytes));
            }
        }
        let manifest = manifest.ok_or(ReportError::Missing(MANIFEST_NAME))?;
        Bundle::from_parts(&manifest, parts)
    }

    /// Writes the bundle into folder `dir` (created if needed). Files of a
    /// kind the bundle doesn't hold are removed; `report.json` is written
    /// last, through a rename, so it never lists a file that isn't there.
    pub fn write_dir(&self, dir: &Path) -> Result<(), ReportError> {
        std::fs::create_dir_all(dir)?;
        for kind in FileKind::ALL {
            let path = dir.join(kind.file_name());
            match self.files.get(&kind) {
                Some(b) => std::fs::write(&path, b)?,
                None => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                },
            }
        }
        let tmp = dir.join(format!("{MANIFEST_NAME}.tmp"));
        std::fs::write(&tmp, self.manifest.to_json())?;
        std::fs::rename(&tmp, dir.join(MANIFEST_NAME))?;
        Ok(())
    }

    /// Adds `bytes` as the file of `kind` to the bundle already in `dir`
    /// (the game's screenshot arrives a frame after the rest).
    pub fn add_to_dir(dir: &Path, kind: FileKind, bytes: Vec<u8>) -> Result<(), ReportError> {
        Bundle::read_dir(dir)?
            .with_file(kind, bytes)?
            .write_dir(dir)
    }

    fn sync_manifest(&mut self) {
        self.manifest.files = self
            .files
            .iter()
            .map(|(&kind, b)| FileEntry {
                kind,
                size: b.len() as u64,
                sha256: sha256(b),
            })
            .collect();
    }

    fn check_total(&self) -> Result<(), ReportError> {
        let total = self.total_bytes();
        if total > MAX_TOTAL_BYTES {
            return Err(too_large("the bug report", total, MAX_TOTAL_BYTES));
        }
        let json = self.manifest.to_json().len() as u64;
        if json > MAX_MANIFEST_BYTES {
            return Err(too_large(MANIFEST_NAME, json, MAX_MANIFEST_BYTES));
        }
        Ok(())
    }
}

/// One file's own rules: its size limit and the shape of its content.
fn check_content(kind: FileKind, bytes: &[u8]) -> Result<(), ReportError> {
    let name = kind.file_name();
    if bytes.len() as u64 > kind.max_bytes() {
        return Err(too_large(name, bytes.len() as u64, kind.max_bytes()));
    }
    let bad = |why: &str| ReportError::BadContent {
        file: name,
        why: why.into(),
    };
    match kind {
        FileKind::Screenshot => {
            const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
            if !bytes.starts_with(&PNG_SIGNATURE) {
                return Err(bad("not a PNG"));
            }
        }
        FileKind::Log => {
            let text = std::str::from_utf8(bytes).map_err(|_| bad("not UTF-8 text"))?;
            let lines = text.lines().count();
            if lines > MAX_LOG_LINES {
                return Err(ReportError::TooMany {
                    what: name.into(),
                    count: lines,
                    max: MAX_LOG_LINES,
                    unit: "lines",
                });
            }
        }
        FileKind::State => {
            StateDump::parse(bytes)?;
        }
    }
    Ok(())
}

fn too_large(what: &str, size: u64, max: u64) -> ReportError {
    ReportError::TooLarge {
        what: crate::clip(what, MAX_LABEL_CHARS),
        size,
        max,
    }
}

/// Reads at most `max` bytes of `path`; a file that grew past its checked
/// size since is refused.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, ReportError> {
    use std::io::Read;
    let mut out = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(too_large(
            &path.file_name().unwrap_or_default().to_string_lossy(),
            out.len() as u64,
            max,
        ));
    }
    Ok(out)
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

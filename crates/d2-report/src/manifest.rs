//! `report.json`: what a bug report says about itself.
//!
//! It names the format and version, the game and launcher versions, the OS,
//! when the report was made, the player's note, and every other file in the
//! bundle with its size and SHA-256. Reading is strict: unknown fields, wrong
//! types, a foreign format or an unknown version are all rejected, so a
//! reader never guesses at what a field meant.

use serde_json::{json, Map, Value};

use crate::{
    FileKind, ReportError, FORMAT, MAX_LABEL_CHARS, MAX_MANIFEST_BYTES, MAX_NOTE_CHARS, VERSION,
};

/// One listed file: its kind (which fixes its name), size and hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub kind: FileKind,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// The parsed `report.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The game's version (`D2RUST_VERSION`, four dotted numbers).
    pub game_version: String,
    /// The launcher's version, when the launcher added it.
    pub launcher_version: Option<String>,
    /// The OS the game ran on (`windows`, `linux`, ...).
    pub os: String,
    /// When the report was made, in seconds since the Unix epoch.
    pub time_unix: u64,
    /// What the player wrote, as written (untrusted).
    pub note: String,
    /// The bundle's other files, in [`FileKind::ALL`] order.
    pub files: Vec<FileEntry>,
}

const KEYS: [&str; 8] = [
    "format",
    "version",
    "game_version",
    "launcher_version",
    "os",
    "time_unix",
    "note",
    "files",
];

impl Manifest {
    /// The listed entry of `kind`, if any.
    pub fn entry(&self, kind: FileKind) -> Option<&FileEntry> {
        self.files.iter().find(|e| e.kind == kind)
    }

    /// The text fields' own limits (the files are checked with their bytes).
    pub fn check_fields(&self) -> Result<(), ReportError> {
        label("game_version", &self.game_version, false)?;
        if let Some(l) = &self.launcher_version {
            label("launcher_version", l, false)?;
        }
        label("os", &self.os, true)?;
        let n = self.note.chars().count();
        if n > MAX_NOTE_CHARS {
            return Err(ReportError::TooMany {
                what: "the note".into(),
                count: n,
                max: MAX_NOTE_CHARS,
                unit: "characters",
            });
        }
        let mut last = None;
        for e in &self.files {
            if Some(e.kind) == last {
                return Err(ReportError::Duplicate(e.kind.file_name()));
            }
            if last.is_some_and(|l| l > e.kind) {
                return Err(ReportError::Malformed("files are out of order".into()));
            }
            last = Some(e.kind);
        }
        Ok(())
    }

    /// `report.json`'s bytes (pretty JSON, so a person can read it too).
    pub fn to_json(&self) -> Vec<u8> {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|e| {
                json!({
                    "kind": e.kind.as_str(),
                    "name": e.kind.file_name(),
                    "size": e.size,
                    "sha256": hex(&e.sha256),
                })
            })
            .collect();
        let v = json!({
            "format": FORMAT,
            "version": VERSION,
            "game_version": self.game_version,
            "launcher_version": self.launcher_version,
            "os": self.os,
            "time_unix": self.time_unix,
            "note": self.note,
            "files": files,
        });
        let mut out = serde_json::to_vec_pretty(&v).expect("a JSON value always serialises");
        out.push(b'\n');
        out
    }

    /// Parses `report.json`, strictly; see the module header.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, ReportError> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(ReportError::TooLarge {
                what: crate::MANIFEST_NAME.into(),
                size: bytes.len() as u64,
                max: MAX_MANIFEST_BYTES,
            });
        }
        let v: Value =
            serde_json::from_slice(bytes).map_err(|e| ReportError::Malformed(e.to_string()))?;
        let Value::Object(o) = v else {
            return Err(bad("the top level is not an object"));
        };
        // Format and version first: a newer report gets "unsupported", not a
        // complaint about a field this version doesn't know.
        let format = o.get("format").and_then(Value::as_str).unwrap_or("");
        let version = o.get("version").and_then(Value::as_u64).unwrap_or(0);
        if format != FORMAT || version != u64::from(VERSION) {
            return Err(ReportError::Unsupported {
                format: crate::clip(format, MAX_LABEL_CHARS),
                version,
            });
        }
        if let Some(k) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return Err(bad(&format!(
                "unknown field {:?}",
                crate::clip(k, MAX_LABEL_CHARS)
            )));
        }
        let launcher_version = match o.get("launcher_version") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(bad("launcher_version is not a string")),
        };
        let files = match o.get("files") {
            Some(Value::Array(a)) => a.iter().map(entry).collect::<Result<Vec<_>, _>>()?,
            _ => return Err(bad("files is missing or not a list")),
        };
        let m = Manifest {
            game_version: string(&o, "game_version")?,
            launcher_version,
            os: string(&o, "os")?,
            time_unix: o
                .get("time_unix")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad("time_unix is missing or not a whole number"))?,
            note: string(&o, "note")?,
            files,
        };
        m.check_fields()?;
        Ok(m)
    }
}

fn bad(why: &str) -> ReportError {
    ReportError::Malformed(why.into())
}

fn string(o: &Map<String, Value>, key: &str) -> Result<String, ReportError> {
    o.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(&format!("{key} is missing or not a string")))
}

/// A short label: printable ASCII, not empty, at most [`MAX_LABEL_CHARS`];
/// `word` labels (the OS) allow only letters, digits, `-` and `_`.
fn label(what: &str, s: &str, word: bool) -> Result<(), ReportError> {
    let ok_char = |c: char| {
        if word {
            c.is_ascii_alphanumeric() || c == '-' || c == '_'
        } else {
            c.is_ascii_graphic() || c == ' '
        }
    };
    if s.is_empty() || s.len() > MAX_LABEL_CHARS || !s.chars().all(ok_char) {
        return Err(bad(&format!(
            "{what} must be 1 to {MAX_LABEL_CHARS} plain characters"
        )));
    }
    Ok(())
}

fn entry(v: &Value) -> Result<FileEntry, ReportError> {
    let Value::Object(o) = v else {
        return Err(bad("a files entry is not an object"));
    };
    if let Some(k) = o
        .keys()
        .find(|k| !["kind", "name", "size", "sha256"].contains(&k.as_str()))
    {
        return Err(bad(&format!(
            "unknown field {:?} in a files entry",
            crate::clip(k, MAX_LABEL_CHARS)
        )));
    }
    let name = o.get("name").and_then(Value::as_str).unwrap_or("");
    let kind = o
        .get("kind")
        .and_then(Value::as_str)
        .and_then(FileKind::from_str_exact)
        .ok_or_else(|| ReportError::UnknownFile(crate::clip(name, MAX_LABEL_CHARS)))?;
    if name != kind.file_name() {
        return Err(ReportError::UnknownFile(crate::clip(name, MAX_LABEL_CHARS)));
    }
    let size = o
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("a files entry's size is missing or not a whole number"))?;
    let sha256 = o
        .get("sha256")
        .and_then(Value::as_str)
        .and_then(unhex)
        .ok_or_else(|| bad("a files entry's sha256 is not 64 lowercase hex digits"))?;
    Ok(FileEntry { kind, size, sha256 })
}

pub(crate) fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 || !b.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

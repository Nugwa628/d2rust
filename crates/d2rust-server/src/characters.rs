//! The character folder: online characters, stored on the server.
//!
//! Each character is one file, `<name>.d2rc`, in D2RUST's own versioned
//! character format. The file is an envelope around the character record
//! that the game's save step writes:
//!
//! | Bytes | Field |
//! |---|---|
//! | 4 | magic `D2RC` |
//! | 2 | format version (little-endian), [`FORMAT_VERSION`] now |
//! | 2 | reserved, 0 |
//! | 4 | record length (little-endian) |
//! | n | the record |
//! | 4 | FNV-1a 32 of everything before it (little-endian) |
//!
//! Reading an older version runs its migrations up to [`FORMAT_VERSION`];
//! a newer version is refused, so an old server never damages a newer
//! file. A save writes a temporary file and renames it over the old one,
//! so a crash mid-save leaves the last good file.
//!
//! Names follow the game's character name rules (see [`check_name`]) and
//! are unique without regard to case, the same on every file system.
//! Characters made on a client never come here, and characters stored
//! here are never sent to a client as files.

use std::path::{Path, PathBuf};

/// The format version this server writes.
pub const FORMAT_VERSION: u16 = 1;

/// The file name extension of a stored character.
pub const EXTENSION: &str = "d2rc";

const MAGIC: &[u8; 4] = b"D2RC";
const HEADER: usize = 12;
const TRAILER: usize = 4;

/// A character store failure.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CharacterError {
    /// The name breaks the name rules.
    #[error("bad character name `{name}`: {reason}")]
    BadName {
        /// The name.
        name: String,
        /// Which rule it breaks.
        reason: &'static str,
    },
    /// A character with this name (in any case) exists already.
    #[error("a character named `{0}` exists already")]
    Exists(String),
    /// No character has this name.
    #[error("no character named `{0}`")]
    NotFound(String),
    /// The file is not a stored character, or it is damaged.
    #[error("{name}: damaged character file: {reason}")]
    Damaged {
        /// The character.
        name: String,
        /// What is wrong.
        reason: &'static str,
    },
    /// The file was written by a newer server.
    #[error("{name}: format version {version} is newer than this server's {FORMAT_VERSION}")]
    TooNew {
        /// The character.
        name: String,
        /// The file's version.
        version: u16,
    },
    /// Reading or writing the folder failed.
    #[error("{0}")]
    Io(String),
}

fn io(e: std::io::Error) -> CharacterError {
    CharacterError::Io(e.to_string())
}

/// Checks a character name: 2 to 15 ASCII letters, with at most one `-`
/// or `_` that is neither first nor last.
pub fn check_name(name: &str) -> Result<(), CharacterError> {
    let bad = |reason| {
        Err(CharacterError::BadName {
            name: name.into(),
            reason,
        })
    };
    if !(2..=15).contains(&name.len()) {
        return bad("it must be 2 to 15 characters long");
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'_')
    {
        return bad("only letters, `-` and `_` are allowed");
    }
    let marks = name.bytes().filter(|&b| b == b'-' || b == b'_').count();
    if marks > 1 {
        return bad("at most one `-` or `_` is allowed");
    }
    let first_or_last = |b: Option<u8>| matches!(b, Some(b'-' | b'_'));
    if first_or_last(name.bytes().next()) || first_or_last(name.bytes().last()) {
        return bad("it can't start or end with `-` or `_`");
    }
    Ok(())
}

/// FNV-1a, 32 bits: the envelope's check against damaged files.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// Wraps a character record in the current envelope.
pub fn encode(record: &[u8]) -> Vec<u8> {
    let len = u32::try_from(record.len()).expect("a character record under 4 GB");
    let mut out = Vec::with_capacity(HEADER + record.len() + TRAILER);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(record);
    out.extend_from_slice(&fnv1a(&out).to_le_bytes());
    out
}

/// Unwraps a stored character of `name`, migrating it to
/// [`FORMAT_VERSION`]. Returns the record.
pub fn decode(name: &str, file: &[u8]) -> Result<Vec<u8>, CharacterError> {
    let damaged = |reason| CharacterError::Damaged {
        name: name.into(),
        reason,
    };
    if file.len() < HEADER + TRAILER || &file[..4] != MAGIC {
        return Err(damaged("not a D2RUST character file"));
    }
    let (body, sum) = file.split_at(file.len() - TRAILER);
    if fnv1a(body).to_le_bytes() != sum {
        return Err(damaged("its checksum doesn't match"));
    }
    let version = u16::from_le_bytes([file[4], file[5]]);
    let len = u32::from_le_bytes([file[8], file[9], file[10], file[11]]) as usize;
    if HEADER + len != body.len() {
        return Err(damaged("its record length doesn't match"));
    }
    let record = body[HEADER..].to_vec();
    migrate(name, version, record)
}

/// Brings a record of format `version` up to [`FORMAT_VERSION`]. Version 1
/// is the first format, so there is nothing to migrate yet; a later
/// format adds one step per version here.
fn migrate(name: &str, version: u16, record: Vec<u8>) -> Result<Vec<u8>, CharacterError> {
    match version {
        FORMAT_VERSION => Ok(record),
        v if v > FORMAT_VERSION => Err(CharacterError::TooNew {
            name: name.into(),
            version: v,
        }),
        _ => Err(CharacterError::Damaged {
            name: name.into(),
            reason: "format version 0 doesn't exist",
        }),
    }
}

/// The character folder.
#[derive(Clone, Debug)]
pub struct CharacterStore {
    dir: PathBuf,
}

impl CharacterStore {
    /// Opens the character folder at `dir`, creating it if needed.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, CharacterError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(io)?;
        Ok(CharacterStore { dir })
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The stored characters' names, sorted without regard to case.
    pub fn list(&self) -> Result<Vec<String>, CharacterError> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if check_name(stem).is_ok() {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort_by_key(|n| n.to_ascii_lowercase());
        Ok(names)
    }

    /// The stored name matching `name` without regard to case.
    fn find(&self, name: &str) -> Result<Option<String>, CharacterError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|n| n.eq_ignore_ascii_case(name)))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.{EXTENSION}"))
    }

    /// Stores a new character. Fails if the name breaks the rules or is
    /// taken (in any case).
    pub fn create(&self, name: &str, record: &[u8]) -> Result<(), CharacterError> {
        check_name(name)?;
        if let Some(taken) = self.find(name)? {
            return Err(CharacterError::Exists(taken));
        }
        self.write(name, record)
    }

    /// Reads a character's record; `name` matches without regard to case.
    pub fn load(&self, name: &str) -> Result<Vec<u8>, CharacterError> {
        let stored = self
            .find(name)?
            .ok_or_else(|| CharacterError::NotFound(name.into()))?;
        let file = std::fs::read(self.path(&stored)).map_err(io)?;
        decode(&stored, &file)
    }

    /// Saves an existing character's record over its file.
    pub fn save(&self, name: &str, record: &[u8]) -> Result<(), CharacterError> {
        let stored = self
            .find(name)?
            .ok_or_else(|| CharacterError::NotFound(name.into()))?;
        self.write(&stored, record)
    }

    /// Writes `<name>.d2rc.tmp`, flushes it to disk, then renames it over
    /// `<name>.d2rc`.
    fn write(&self, name: &str, record: &[u8]) -> Result<(), CharacterError> {
        use std::io::Write;
        let path = self.path(name);
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        {
            let mut f = std::fs::File::create(&tmp).map_err(io)?;
            f.write_all(&encode(record)).map_err(io)?;
            f.sync_all().map_err(io)?;
        }
        std::fs::rename(&tmp, &path).map_err(io)
    }
}

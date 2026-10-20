//! The bug report bundle, format version 1.
//!
//! A bug report is one folder (or, on the way to the report service, one
//! upload) holding `report.json` and the files it lists. The game's
//! `bugreport` command writes it; the launcher reads it to show the player
//! what will be sent; the report service checks it again on arrival. All
//! three use this crate, so a bundle one side accepts the others accept too.
//!
//! The rules:
//! - Everything in a report is untrusted data. This crate only checks shape
//!   and limits; it never interprets the note, the log or the state text.
//! - File names are fixed (one per [`FileKind`]), so a bundle can never name
//!   a path of its own choosing.
//! - Every limit is checked on build, on read and on upload alike, and a
//!   rejected bundle says why ([`ReportError`]).
//! - `report.json` carries a format name and version from the first version
//!   on; a reader rejects versions it doesn't know.
//!
//! No Bevy and no game logic here: the crate is shared with tools outside the
//! game.

mod bundle;
mod manifest;
pub mod privacy;
mod screenshot;
pub mod state;

pub use bundle::{Bundle, Meta};
pub use manifest::{FileEntry, Manifest};
pub use privacy::Redactor;
pub use screenshot::screenshot_png;
pub use state::{state_text, StateDump};

/// `report.json`'s `format` field.
pub const FORMAT: &str = "d2rust-bug-report";
/// The format version this crate writes and reads.
pub const VERSION: u32 = 1;
/// The manifest's file name inside a bundle.
pub const MANIFEST_NAME: &str = "report.json";

/// The whole bundle, manifest included.
pub const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
/// `report.json` itself.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The screenshot PNG.
pub const MAX_SCREENSHOT_BYTES: u64 = 4 * 1024 * 1024;
/// The log tail: at most this many lines...
pub const MAX_LOG_LINES: usize = 2000;
/// ...and at most this many bytes.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// The state dump.
pub const MAX_STATE_BYTES: u64 = 64 * 1024;
/// The player's note, in characters.
pub const MAX_NOTE_CHARS: usize = 4000;
/// Version strings and the OS name, in characters.
pub const MAX_LABEL_CHARS: usize = 64;

/// The files a bundle may hold besides `report.json`. Each kind has one
/// fixed file name and its own size limit; a bundle holds each at most once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    /// What was on screen, as PNG.
    Screenshot,
    /// The newest lines of the game's log, UTF-8 text.
    Log,
    /// The game's state when the report was made ([`StateDump`]).
    State,
}

impl FileKind {
    /// Every kind, in the order a bundle lists them.
    pub const ALL: [FileKind; 3] = [FileKind::Screenshot, FileKind::Log, FileKind::State];

    /// The kind's name in `report.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Screenshot => "screenshot",
            FileKind::Log => "log",
            FileKind::State => "state",
        }
    }

    /// The kind's file name inside a bundle.
    pub fn file_name(self) -> &'static str {
        match self {
            FileKind::Screenshot => "screenshot.png",
            FileKind::Log => "log.txt",
            FileKind::State => "state.txt",
        }
    }

    /// The largest file of this kind a bundle may hold.
    pub fn max_bytes(self) -> u64 {
        match self {
            FileKind::Screenshot => MAX_SCREENSHOT_BYTES,
            FileKind::Log => MAX_LOG_BYTES,
            FileKind::State => MAX_STATE_BYTES,
        }
    }

    /// The kind named `s` in `report.json`.
    pub fn from_str_exact(s: &str) -> Option<FileKind> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The kind whose file name is exactly `name`.
    pub fn from_file_name(name: &str) -> Option<FileKind> {
        Self::ALL.into_iter().find(|k| k.file_name() == name)
    }
}

/// Why a bundle was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("{what} is {size} bytes, more than the {max} allowed")]
    TooLarge { what: String, size: u64, max: u64 },
    #[error("{what} has {count} {unit}, more than the {max} allowed")]
    TooMany {
        what: String,
        count: usize,
        max: usize,
        unit: &'static str,
    },
    #[error("report.json is malformed: {0}")]
    Malformed(String),
    #[error("report format {format:?} version {version} is not supported (this reads {FORMAT:?} version {VERSION})")]
    Unsupported { format: String, version: u64 },
    #[error("{0:?} is not a file a bug report may hold")]
    UnknownFile(String),
    #[error("{0} is listed in report.json but missing")]
    Missing(&'static str),
    #[error("{0} is present but not listed in report.json")]
    Unlisted(&'static str),
    #[error("{0} appears more than once")]
    Duplicate(&'static str),
    #[error("{file} doesn't match report.json: {what}")]
    Mismatch {
        file: &'static str,
        what: &'static str,
    },
    #[error("{file} is not valid: {why}")]
    BadContent { file: &'static str, why: String },
    #[error("reading or writing the report: {0}")]
    Io(#[from] std::io::Error),
}

/// `text` cut to at most `max` characters (at a character boundary).
pub fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The log tail a bundle holds: the last [`MAX_LOG_LINES`] of `lines`,
/// newline-terminated, with the oldest lines dropped further until the text
/// fits [`MAX_LOG_BYTES`]. Over-long single lines are cut.
pub fn log_tail<S: AsRef<str>>(lines: &[S]) -> Vec<u8> {
    const MAX_LINE_CHARS: usize = 4000;
    let start = lines.len().saturating_sub(MAX_LOG_LINES);
    let mut kept: Vec<String> = lines[start..]
        .iter()
        .map(|l| clip(&l.as_ref().replace(['\r', '\n'], " "), MAX_LINE_CHARS) + "\n")
        .collect();
    let mut total: usize = kept.iter().map(String::len).sum();
    let mut drop = 0;
    while total as u64 > MAX_LOG_BYTES {
        total -= kept[drop].len();
        drop += 1;
    }
    kept.drain(..drop);
    kept.concat().into_bytes()
}

#[cfg(test)]
mod tests;

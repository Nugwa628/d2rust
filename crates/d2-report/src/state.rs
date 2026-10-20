//! The state dump inside a bug report (`state.txt`).
//!
//! The game writes what it knows about the moment of the report as short
//! text lines: the build, the character's save name, the area, position,
//! life, server tick, seed and the hovered unit (the same lines the in-game
//! `info` overlay shows). The file starts with a versioned header line so a
//! reader can tell the dump's layout apart from any later one.
//!
//! Parsing is strict: the exact header, at most [`MAX_STATE_LINES`] lines,
//! each at most [`MAX_STATE_LINE_CHARS`] characters, no control characters.
//! Anything else is rejected rather than repaired.

use crate::{FileKind, ReportError, MAX_STATE_BYTES};

/// The dump's first line: its layout and version.
pub const STATE_HEADER: &str = "d2rust-state 1";
/// Lines after the header.
pub const MAX_STATE_LINES: usize = 64;
/// Characters per line.
pub const MAX_STATE_LINE_CHARS: usize = 200;

/// A parsed state dump: the lines after the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateDump {
    pub lines: Vec<String>,
}

impl StateDump {
    /// Parses `state.txt`; see the module header for the rules.
    pub fn parse(bytes: &[u8]) -> Result<StateDump, ReportError> {
        let bad = |why: String| ReportError::BadContent {
            file: FileKind::State.file_name(),
            why,
        };
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(ReportError::TooLarge {
                what: FileKind::State.file_name().into(),
                size: bytes.len() as u64,
                max: MAX_STATE_BYTES,
            });
        }
        let text = std::str::from_utf8(bytes).map_err(|_| bad("not UTF-8 text".into()))?;
        let body = text
            .strip_suffix('\n')
            .ok_or_else(|| bad("doesn't end with a newline".into()))?;
        let mut it = body.split('\n');
        if it.next() != Some(STATE_HEADER) {
            return Err(bad(format!("the first line isn't {STATE_HEADER:?}")));
        }
        let lines: Vec<String> = it.map(str::to_owned).collect();
        if lines.len() > MAX_STATE_LINES {
            return Err(ReportError::TooMany {
                what: FileKind::State.file_name().into(),
                count: lines.len(),
                max: MAX_STATE_LINES,
                unit: "lines",
            });
        }
        for (i, l) in lines.iter().enumerate() {
            if l.chars().count() > MAX_STATE_LINE_CHARS {
                return Err(bad(format!("line {} is too long", i + 2)));
            }
            if l.chars().any(char::is_control) {
                return Err(bad(format!("line {} has control characters", i + 2)));
            }
        }
        Ok(StateDump { lines })
    }
}

/// `state.txt`'s bytes for `lines`. Lines are made to fit the format (control
/// characters become spaces, long lines are cut, extra lines are dropped), so
/// what the game writes always parses.
pub fn state_text<S: AsRef<str>>(lines: &[S]) -> Vec<u8> {
    let mut s = String::from(STATE_HEADER);
    s.push('\n');
    for l in lines.iter().take(MAX_STATE_LINES) {
        let clean: String = l
            .as_ref()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_STATE_LINE_CHARS)
            .collect();
        s.push_str(&clean);
        s.push('\n');
    }
    s.into_bytes()
}

//! Voice log (an exactness check): one record per voice
//! start, stop and parameter change, written as JSON lines under the header
//! `{"format":"d2rust-audio-log","version":1}`. The parser accepts
//! exactly what the writer produces and nothing else.
//!
//! Record line, keys in this order, no whitespace:
//! `{"tick":N,"kind":"start|stop|param","file":S,"vol":N,"pan":N,"looped":B,"error":B,"cause":S}`.
//! Strings escape `"` and `\` with a backslash and every char below U+0020
//! as `\u00XX` (lowercase hex); no other escape occurs. Every line ends
//! with `\n`.

use std::fmt::Write as _;

/// Format name in the header.
pub const LOG_FORMAT: &str = "d2rust-audio-log";
/// Format version.
pub const LOG_VERSION: u32 = 1;

/// The exact header line, without its `\n`.
pub fn header() -> String {
    format!("{{\"format\":\"{LOG_FORMAT}\",\"version\":{LOG_VERSION}}}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    Start,
    Stop,
    Param,
}

impl VoiceKind {
    fn as_str(self) -> &'static str {
        match self {
            VoiceKind::Start => "start",
            VoiceKind::Stop => "stop",
            VoiceKind::Param => "param",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(VoiceKind::Start),
            "stop" => Some(VoiceKind::Stop),
            "param" => Some(VoiceKind::Param),
            _ => None,
        }
    }
}

/// One voice log record. `error` marks a start whose
/// sound could not play. Compared: `tick`, `kind`,
/// `file`, `vol`, `pan`, `looped`. Ours, not compared: `error`, `cause`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceEvent {
    pub tick: u32,
    pub kind: VoiceKind,
    pub file: String,
    pub vol: i32,
    pub pan: i32,
    pub looped: bool,
    pub error: bool,
    pub cause: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoiceLog {
    pub events: Vec<VoiceEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogError {
    #[error("empty audio log (no header)")]
    Empty,
    #[error("line 1: header is not {expected}")]
    Header { expected: String },
    #[error("line {line}: {what}")]
    Record { line: usize, what: String },
    #[error("log does not end with a newline")]
    NoFinalNewline,
}

impl VoiceLog {
    pub fn push(&mut self, e: VoiceEvent) {
        self.events.push(e);
    }

    /// Serialize as JSON lines, header first.
    pub fn to_jsonl(&self) -> String {
        let mut s = header();
        s.push('\n');
        for e in &self.events {
            let _ = write!(
                s,
                "{{\"tick\":{},\"kind\":\"{}\",\"file\":",
                e.tick,
                e.kind.as_str()
            );
            push_str(&mut s, &e.file);
            let _ = write!(
                s,
                ",\"vol\":{},\"pan\":{},\"looped\":{},\"error\":{},\"cause\":",
                e.vol, e.pan, e.looped, e.error
            );
            push_str(&mut s, &e.cause);
            s.push_str("}\n");
        }
        s
    }

    /// Strict parse of [`to_jsonl`](Self::to_jsonl) output.
    pub fn parse(text: &str) -> Result<VoiceLog, LogError> {
        if text.is_empty() {
            return Err(LogError::Empty);
        }
        let Some(body) = text.strip_suffix('\n') else {
            return Err(LogError::NoFinalNewline);
        };
        let mut lines = body.split('\n');
        let expected = header();
        if lines.next() != Some(expected.as_str()) {
            return Err(LogError::Header { expected });
        }
        let mut events = Vec::new();
        for (i, line) in lines.enumerate() {
            let line_no = i + 2;
            let e = parse_record(line).map_err(|what| LogError::Record {
                line: line_no,
                what,
            })?;
            events.push(e);
        }
        Ok(VoiceLog { events })
    }
}

fn push_str(s: &mut String, v: &str) {
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
}

struct Cursor<'a> {
    rest: &'a str,
}

impl<'a> Cursor<'a> {
    fn lit(&mut self, l: &str) -> Result<(), String> {
        match self.rest.strip_prefix(l) {
            Some(r) => {
                self.rest = r;
                Ok(())
            }
            None => Err(format!("expected {l:?} at {:?}", self.rest)),
        }
    }

    /// Optional `-`, then digits without a leading zero (except `0`).
    fn int(&mut self) -> Result<i64, String> {
        let neg = self.rest.starts_with('-');
        let digits_from = usize::from(neg);
        let len = self.rest[digits_from..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let digits = &self.rest[digits_from..digits_from + len];
        if digits.is_empty()
            || (digits.len() > 1 && digits.starts_with('0'))
            || (neg && digits == "0")
        {
            return Err(format!("bad integer at {:?}", self.rest));
        }
        let v: i64 = self.rest[..digits_from + len]
            .parse()
            .map_err(|_| format!("integer out of range at {:?}", self.rest))?;
        self.rest = &self.rest[digits_from + len..];
        Ok(v)
    }

    fn bool(&mut self) -> Result<bool, String> {
        if self.lit("true").is_ok() {
            Ok(true)
        } else if self.lit("false").is_ok() {
            Ok(false)
        } else {
            Err(format!("expected true or false at {:?}", self.rest))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.lit("\"")?;
        let mut out = String::new();
        let mut chars = self.rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    self.rest = &self.rest[i + 1..];
                    return Ok(out);
                }
                '\\' => match chars.next() {
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((j, 'u')) => {
                        let hex = self.rest.get(j + 1..j + 5).unwrap_or("");
                        let v = (hex.len() == 4
                            && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
                        .then(|| u32::from_str_radix(hex, 16).ok())
                        .flatten()
                        .filter(|v| *v < 0x20)
                        .ok_or_else(|| format!("bad \\u escape {hex:?}"))?;
                        out.push(char::from_u32(v).unwrap_or('\0'));
                        for _ in 0..4 {
                            chars.next();
                        }
                    }
                    _ => return Err("bad escape".to_owned()),
                },
                c if (c as u32) < 0x20 => return Err("unescaped control char".to_owned()),
                c => out.push(c),
            }
        }
        Err("unterminated string".to_owned())
    }
}

fn parse_record(line: &str) -> Result<VoiceEvent, String> {
    let mut c = Cursor { rest: line };
    let narrow = |v: i64, what: &str| -> Result<i32, String> {
        i32::try_from(v).map_err(|_| format!("{what} {v} out of range"))
    };
    c.lit("{\"tick\":")?;
    let tick_raw = c.int()?;
    let tick = u32::try_from(tick_raw).map_err(|_| format!("tick {tick_raw} out of range"))?;
    c.lit(",\"kind\":")?;
    let kind_s = c.string()?;
    let kind = VoiceKind::parse(&kind_s).ok_or_else(|| format!("unknown kind {kind_s:?}"))?;
    c.lit(",\"file\":")?;
    let file = c.string()?;
    c.lit(",\"vol\":")?;
    let vol = narrow(c.int()?, "vol")?;
    c.lit(",\"pan\":")?;
    let pan = narrow(c.int()?, "pan")?;
    c.lit(",\"looped\":")?;
    let looped = c.bool()?;
    c.lit(",\"error\":")?;
    let error = c.bool()?;
    c.lit(",\"cause\":")?;
    let cause = c.string()?;
    c.lit("}")?;
    if !c.rest.is_empty() {
        return Err(format!("trailing text {:?}", c.rest));
    }
    Ok(VoiceEvent {
        tick,
        kind,
        file,
        vol,
        pan,
        looped,
        error,
        cause,
    })
}

/// One difference between two logs under the log comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogMismatch {
    /// Record `index` differs in these compared fields.
    Field {
        index: usize,
        fields: Vec<&'static str>,
    },
    /// The logs have different lengths (records past the shorter one are
    /// not compared field by field).
    Length { ours: usize, theirs: usize },
}

/// Compare two logs as identical sequences of
/// `(tick, kind, file, vol, pan, looped)`. `cause` and
/// `error` are ours and not compared. Empty result: identical.
pub fn compare_logs(ours: &VoiceLog, theirs: &VoiceLog) -> Vec<LogMismatch> {
    let mut out = Vec::new();
    for (index, (a, b)) in ours.events.iter().zip(&theirs.events).enumerate() {
        let mut fields = Vec::new();
        if a.tick != b.tick {
            fields.push("tick");
        }
        if a.kind != b.kind {
            fields.push("kind");
        }
        if a.file != b.file {
            fields.push("file");
        }
        if a.vol != b.vol {
            fields.push("vol");
        }
        if a.pan != b.pan {
            fields.push("pan");
        }
        if a.looped != b.looped {
            fields.push("looped");
        }
        if !fields.is_empty() {
            out.push(LogMismatch::Field { index, fields });
        }
    }
    if ours.events.len() != theirs.events.len() {
        out.push(LogMismatch::Length {
            ours: ours.events.len(),
            theirs: theirs.events.len(),
        });
    }
    out
}

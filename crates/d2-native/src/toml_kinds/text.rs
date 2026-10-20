//! The toolkit every text kind is built from: the error type, a
//! line-by-line TOML emitter ([`Out`]), a strict TOML reader ([`Tab`]),
//! TSV rows and cell escapes, and the spellings of bytes, hex numbers and
//! Latin-1 names.

use std::fmt::{Debug, Display, Write as _};

use crate::native_toml::parse_hex;

/// `native_version` of every text kind.
pub const NATIVE_VERSION: i64 = 1;

/// A native read, write or check failure, naming the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {detail}")]
pub struct TextError {
    /// The file that failed.
    pub file: String,
    /// What is wrong with it.
    pub detail: String,
}

impl TextError {
    /// An error for `file` with the given detail.
    pub fn new(file: &str, detail: impl Into<String>) -> TextError {
        TextError {
            file: file.to_owned(),
            detail: detail.into(),
        }
    }
}

/// A decoded value a writer cannot represent: refused rather than written
/// with part of it lost.
pub(super) fn not_representable(file: &str, detail: impl Into<String>) -> TextError {
    TextError::new(file, format!("not representable: {}", detail.into()))
}

/// The round-trip check of a text kind: write `original`, read the text
/// back, and compare; a mismatch names the first differing line.
pub(crate) fn check_round_trip<T: PartialEq + Debug>(
    file: &str,
    original: &T,
    write: fn(&str, &T) -> Result<String, TextError>,
    read: fn(&str, &str) -> Result<T, TextError>,
) -> Result<(), TextError> {
    let back = read(file, &write(file, original)?)?;
    if &back == original {
        Ok(())
    } else {
        Err(TextError::new(file, first_difference(original, &back)))
    }
}

// ---------------------------------------------------------------- spellings

/// `s` as a TOML basic string, quotes included.
fn toml_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Names in the game files are bytes, not text: each byte `b` is spelled
/// as the character U+00`b`, so every name survives the trip through a
/// UTF-8 text file unchanged.
pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The reverse of [`latin1`]; a character above U+00FF is an error.
pub(crate) fn unlatin1(file: &str, what: &str, s: &str) -> Result<Vec<u8>, TextError> {
    s.chars()
        .map(|c| {
            u8::try_from(u32::from(c)).map_err(|_| {
                TextError::new(
                    file,
                    format!("{what}: character U+{:04X} above U+00FF", c as u32),
                )
            })
        })
        .collect()
}

/// Exactly `digits` lowercase hex digits, no prefix.
pub(super) fn parse_bare_hex(s: &str, digits: usize) -> Option<u64> {
    parse_hex(&format!("0x{s}"), digits)
}

/// Bytes as lowercase hex pairs, no prefix (`00abff`).
pub(crate) fn hex_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The reverse of [`hex_bytes`]; anything but lowercase hex pairs is `None`.
pub(crate) fn unhex_bytes(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| parse_bare_hex(s.get(i..i + 2)?, 2).map(|v| v as u8))
        .collect()
}

/// The first line where the pretty `Debug` forms of `a` and `b` differ:
/// a field-level pointer into two possibly large values.
pub(crate) fn first_difference<T: Debug>(a: &T, b: &T) -> String {
    let (sa, sb) = (format!("{a:#?}"), format!("{b:#?}"));
    let (mut la, mut lb) = (sa.lines(), sb.lines());
    let mut n = 1;
    loop {
        match (la.next(), lb.next()) {
            (Some(x), Some(y)) if x == y => n += 1,
            (x, y) => {
                return format!(
                    "first difference at debug line {n}: original `{}`, native `{}`",
                    x.unwrap_or("<end>").trim(),
                    y.unwrap_or("<end>").trim()
                )
            }
        }
    }
}

// ---------------------------------------------------------------- writing

/// A TOML emitter, one `key = value` line at a time, so keys come out in
/// exactly the order the writer emits them.
#[derive(Default)]
pub(crate) struct Out(pub String);

impl Out {
    /// A new file starting with its `native` kind and `native_version`.
    pub fn header(kind: &str) -> Out {
        let mut o = Out::default();
        o.str("native", kind);
        o.int("native_version", NATIVE_VERSION);
        o
    }
    /// A line as it is.
    pub fn line(&mut self, s: &str) {
        self.0.push_str(s);
        self.0.push('\n');
    }
    /// `key = value` with the value already spelled.
    pub fn raw(&mut self, key: &str, value: impl Display) {
        let _ = writeln!(self.0, "{key} = {value}");
    }
    pub fn int(&mut self, key: &str, v: impl Display) {
        self.raw(key, v);
    }
    pub fn boolean(&mut self, key: &str, v: bool) {
        self.raw(key, v);
    }
    pub fn str(&mut self, key: &str, v: &str) {
        self.raw(key, toml_str(v));
    }
    /// `"0x…"` with exactly `digits` lowercase hex digits.
    pub fn hex(&mut self, key: &str, v: u64, digits: usize) {
        self.raw(key, format!("\"0x{v:0digits$x}\""));
    }
    pub fn ints<I: IntoIterator<Item = T>, T: Display>(&mut self, key: &str, v: I) {
        let items: Vec<String> = v.into_iter().map(|x| x.to_string()).collect();
        self.raw(key, format!("[{}]", items.join(", ")));
    }
    pub fn strs<'a, I: IntoIterator<Item = &'a str>>(&mut self, key: &str, v: I) {
        self.raw(key, toml_array(v.into_iter().map(toml_str)));
    }
    /// A blank line, then a table header such as `[[layer]]`.
    pub fn table(&mut self, name: &str) {
        self.line("");
        self.line(name);
    }
}

/// `[a, b, c]` of already spelled values.
pub(super) fn toml_array(items: impl Iterator<Item = String>) -> String {
    format!("[{}]", items.collect::<Vec<_>>().join(", "))
}

/// Strings as one TOML array of strings.
pub(super) fn toml_strs(items: impl Iterator<Item = String>) -> String {
    toml_array(items.map(|s| toml_str(&s)))
}

// ---------------------------------------------------------------- reading

/// A TOML table being consumed: every key read is removed, and
/// [`Tab::finish`] fails on any key left over, so an unknown key is an
/// error rather than silently ignored.
pub(crate) struct Tab {
    file: String,
    /// Where this table sits in the file (`[[layer]] #2`), for messages.
    ctx: String,
    t: toml::Table,
}

impl Tab {
    /// Parses a sidecar and checks its `native` kind and `native_version`.
    pub fn parse(file: &str, text: &str, kind: &str) -> Result<Tab, TextError> {
        let t: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| TextError::new(file, format!("not valid TOML: {e}")))?;
        let mut tab = Tab {
            file: file.to_owned(),
            ctx: String::new(),
            t,
        };
        let found = tab.string("native")?;
        if found != kind {
            return Err(tab.err(format!("`native = {found:?}`, expected {kind:?}")));
        }
        let v = tab.int("native_version", i64::MIN, i64::MAX)?;
        if v != NATIVE_VERSION {
            return Err(tab.err(format!("unknown native_version {v}")));
        }
        Ok(tab)
    }

    /// An error in this file, prefixed with where this table sits.
    pub fn err(&self, detail: impl Display) -> TextError {
        let at = if self.ctx.is_empty() {
            String::new()
        } else {
            format!("{}: ", self.ctx)
        };
        TextError::new(&self.file, format!("{at}{detail}"))
    }

    /// Removes and returns `key`, which must be present.
    pub fn take(&mut self, key: &str) -> Result<toml::Value, TextError> {
        self.t
            .remove(key)
            .ok_or_else(|| self.err(format!("missing key `{key}`")))
    }

    pub fn has(&self, key: &str) -> bool {
        self.t.contains_key(key)
    }

    /// An integer in `min..=max`.
    pub fn int(&mut self, key: &str, min: i64, max: i64) -> Result<i64, TextError> {
        match self.take(key)? {
            toml::Value::Integer(v) if (min..=max).contains(&v) => Ok(v),
            toml::Value::Integer(v) => Err(self.err(format!("`{key}` = {v} out of range"))),
            _ => Err(self.err(format!("`{key}` is not an integer"))),
        }
    }
    pub fn u8(&mut self, key: &str) -> Result<u8, TextError> {
        Ok(self.int(key, 0, i64::from(u8::MAX))? as u8)
    }
    pub fn u16(&mut self, key: &str) -> Result<u16, TextError> {
        Ok(self.int(key, 0, i64::from(u16::MAX))? as u16)
    }
    pub fn u32(&mut self, key: &str) -> Result<u32, TextError> {
        Ok(self.int(key, 0, i64::from(u32::MAX))? as u32)
    }
    pub fn i32(&mut self, key: &str) -> Result<i32, TextError> {
        Ok(self.int(key, i64::from(i32::MIN), i64::from(i32::MAX))? as i32)
    }
    pub fn string(&mut self, key: &str) -> Result<String, TextError> {
        match self.take(key)? {
            toml::Value::String(s) => Ok(s),
            _ => Err(self.err(format!("`{key}` is not a string"))),
        }
    }
    pub fn boolean(&mut self, key: &str) -> Result<bool, TextError> {
        match self.take(key)? {
            toml::Value::Boolean(b) => Ok(b),
            _ => Err(self.err(format!("`{key}` is not a boolean"))),
        }
    }
    /// `"0x…"` with exactly `digits` lowercase hex digits.
    pub fn hex(&mut self, key: &str, digits: usize) -> Result<u64, TextError> {
        let s = self.string(key)?;
        parse_hex(&s, digits).ok_or_else(|| {
            self.err(format!(
                "`{key}` = {s:?}: expected \"0x\" and exactly {digits} lowercase hex digits"
            ))
        })
    }
    /// An array of integers, each in `min..=max`.
    pub fn int_array(&mut self, key: &str, min: i64, max: i64) -> Result<Vec<i64>, TextError> {
        match self.take(key)? {
            toml::Value::Array(a) => a
                .into_iter()
                .map(|v| match v {
                    toml::Value::Integer(i) if (min..=max).contains(&i) => Ok(i),
                    _ => Err(self.err(format!("`{key}`: element is not an integer in range"))),
                })
                .collect(),
            _ => Err(self.err(format!("`{key}` is not an array"))),
        }
    }
    pub fn u8_array(&mut self, key: &str) -> Result<Vec<u8>, TextError> {
        Ok(self
            .int_array(key, 0, i64::from(u8::MAX))?
            .into_iter()
            .map(|v| v as u8)
            .collect())
    }
    pub fn str_array(&mut self, key: &str) -> Result<Vec<String>, TextError> {
        match self.take(key)? {
            toml::Value::Array(a) => a
                .into_iter()
                .map(|v| match v {
                    toml::Value::String(s) => Ok(s),
                    _ => Err(self.err(format!("`{key}`: element is not a string"))),
                })
                .collect(),
            _ => Err(self.err(format!("`{key}` is not an array"))),
        }
    }
    /// An array of tables (`[[key]]`); a missing key is an empty list.
    pub fn tables(&mut self, key: &str) -> Result<Vec<Tab>, TextError> {
        let Some(v) = self.t.remove(key) else {
            return Ok(Vec::new());
        };
        let toml::Value::Array(a) = v else {
            return Err(self.err(format!("`{key}` is not an array of tables")));
        };
        a.into_iter()
            .enumerate()
            .map(|(i, v)| match v {
                toml::Value::Table(t) => Ok(self.child(format!("[[{key}]] #{i}"), t)),
                _ => Err(self.err(format!("`{key}` #{i} is not a table"))),
            })
            .collect()
    }
    /// A sub-table (`[key]`) if present.
    pub fn table_opt(&mut self, key: &str) -> Result<Option<Tab>, TextError> {
        match self.t.remove(key) {
            None => Ok(None),
            Some(toml::Value::Table(t)) => Ok(Some(self.child(format!("[{key}]"), t))),
            Some(_) => Err(self.err(format!("`{key}` is not a table"))),
        }
    }
    /// A sub-table of this one, named `name` in messages after this
    /// table's own place.
    fn child(&self, name: String, t: toml::Table) -> Tab {
        let ctx = if self.ctx.is_empty() {
            name
        } else {
            format!("{} {name}", self.ctx)
        };
        Tab {
            file: self.file.clone(),
            ctx,
            t,
        }
    }
    /// Fails on any key that was not read.
    pub fn finish(self) -> Result<(), TextError> {
        match self.t.keys().next() {
            None => Ok(()),
            Some(k) => Err(self.err(format!("unknown key `{k}`"))),
        }
    }
}

// -------------------------------------------------------------------- TSV

/// Escapes one TSV cell: `\`, tab, LF and CR only.
pub(crate) fn tsv_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

/// The reverse of [`tsv_escape`]; any other escape is an error.
pub(crate) fn tsv_unescape(file: &str, line: usize, s: &str) -> Result<String, TextError> {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('\\') => o.push('\\'),
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            other => {
                return Err(TextError::new(
                    file,
                    format!("line {line}: bad escape `\\{}`", other.unwrap_or(' ')),
                ))
            }
        }
    }
    Ok(o)
}

/// The rows of a TSV file as cells (still escaped), after checking its
/// header row and that every row has `columns` cells. The file must end
/// with `\n`; a CR line end shows up as part of the last cell.
pub(crate) fn tsv_rows<'a>(
    file: &str,
    text: &'a str,
    header: &str,
    columns: usize,
) -> Result<Vec<Vec<&'a str>>, TextError> {
    let body = text
        .strip_suffix('\n')
        .ok_or_else(|| TextError::new(file, "no final line end"))?;
    let mut lines = body.split('\n');
    if lines.next() != Some(header) {
        return Err(TextError::new(
            file,
            format!("header is not {:?}", header.replace('\t', "\\t")),
        ));
    }
    lines
        .enumerate()
        .map(|(i, l)| {
            let cells: Vec<&str> = l.split('\t').collect();
            if cells.len() == columns {
                Ok(cells)
            } else {
                Err(TextError::new(
                    file,
                    format!("line {}: {} cells, expected {columns}", i + 2, cells.len()),
                ))
            }
        })
        .collect()
}

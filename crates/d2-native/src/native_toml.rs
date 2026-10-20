//! The TOML toolkit of the picture kinds (`sheet`, `tileset`, `pal`,
//! `expfield`), whose errors are [`NativeError`]s: hex spellings, arrays,
//! and [`Fields`], a strict reader that takes each key once and refuses
//! any key left unread.
//!
//! Every value has exactly one spelling (hex numbers are `"0x"` plus a
//! fixed count of lowercase digits), so a sidecar written twice is
//! byte-identical and an edited value in another spelling is caught. The
//! text kinds have their own reader with the same rules
//! (`toml_kinds`, with [`crate::toml_kinds::TextError`]).

use crate::kind::NativeError;

/// A byte as a quoted lowercase hex string, `"0x0f"`.
pub fn hex2(v: u8) -> String {
    format!("\"0x{v:02x}\"")
}
/// A u16 as a quoted lowercase hex string, `"0x000f"`.
pub fn hex4(v: u16) -> String {
    format!("\"0x{v:04x}\"")
}
/// A u32 as a quoted lowercase hex string, `"0x0000000f"`.
pub fn hex8(v: u32) -> String {
    format!("\"0x{v:08x}\"")
}

/// `[a, b, c]` of already formatted values.
pub fn array<I: IntoIterator<Item = String>>(items: I) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

/// Parses `0x` + exactly `digits` lowercase hex digits.
pub fn parse_hex(s: &str, digits: usize) -> Option<u64> {
    let h = s.strip_prefix("0x")?;
    if h.len() != digits
        || !h
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    u64::from_str_radix(h, 16).ok()
}

/// A TOML table read key by key; [`Fields::finish`] rejects leftovers.
pub struct Fields {
    file: String,
    ctx: String,
    table: toml::Table,
}

impl Fields {
    /// Parses `text` as TOML; `file` names the file in errors.
    pub fn parse(file: &str, text: &str) -> Result<Fields, NativeError> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| NativeError::new(file, format!("bad TOML: {e}")))?;
        Ok(Fields {
            file: file.to_string(),
            ctx: String::new(),
            table,
        })
    }

    /// The file name used in errors.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// An error for this file, prefixed with the sub-table path if any.
    pub fn err(&self, msg: impl std::fmt::Display) -> NativeError {
        let at = if self.ctx.is_empty() {
            String::new()
        } else {
            format!("{}: ", self.ctx)
        };
        NativeError::new(&self.file, format!("{at}{msg}"))
    }

    fn take(&mut self, key: &str) -> Result<toml::Value, NativeError> {
        self.table
            .remove(key)
            .ok_or_else(|| self.err(format!("missing key `{key}`")))
    }

    /// Whether `key` is still unread in this table.
    pub fn has(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }

    /// Reads a string.
    pub fn str(&mut self, key: &str) -> Result<String, NativeError> {
        match self.take(key)? {
            toml::Value::String(s) => Ok(s),
            _ => Err(self.err(format!("`{key}` must be a string"))),
        }
    }

    /// Reads a boolean.
    pub fn bool(&mut self, key: &str) -> Result<bool, NativeError> {
        match self.take(key)? {
            toml::Value::Boolean(b) => Ok(b),
            _ => Err(self.err(format!("`{key}` must be a boolean"))),
        }
    }

    /// Reads an integer.
    pub fn int(&mut self, key: &str) -> Result<i64, NativeError> {
        match self.take(key)? {
            toml::Value::Integer(i) => Ok(i),
            _ => Err(self.err(format!("`{key}` must be an integer"))),
        }
    }

    fn ranged(&mut self, key: &str, lo: i64, hi: i64) -> Result<i64, NativeError> {
        let v = self.int(key)?;
        if v < lo || v > hi {
            return Err(self.err(format!("`{key}` = {v} out of range {lo}..={hi}")));
        }
        Ok(v)
    }

    /// Reads an integer that must fit a u32.
    pub fn u32(&mut self, key: &str) -> Result<u32, NativeError> {
        self.ranged(key, 0, i64::from(u32::MAX)).map(|v| v as u32)
    }
    /// Reads an integer that must fit a u16.
    pub fn u16(&mut self, key: &str) -> Result<u16, NativeError> {
        self.ranged(key, 0, i64::from(u16::MAX)).map(|v| v as u16)
    }
    /// Reads an integer that must fit a u8.
    pub fn u8(&mut self, key: &str) -> Result<u8, NativeError> {
        self.ranged(key, 0, i64::from(u8::MAX)).map(|v| v as u8)
    }
    /// Reads an integer that must fit an i32.
    pub fn i32(&mut self, key: &str) -> Result<i32, NativeError> {
        self.ranged(key, i64::from(i32::MIN), i64::from(i32::MAX))
            .map(|v| v as i32)
    }
    /// Reads an integer that must fit an i16.
    pub fn i16(&mut self, key: &str) -> Result<i16, NativeError> {
        self.ranged(key, i64::from(i16::MIN), i64::from(i16::MAX))
            .map(|v| v as i16)
    }

    fn hex(&mut self, key: &str, digits: usize) -> Result<u64, NativeError> {
        let s = self.str(key)?;
        parse_hex(&s, digits).ok_or_else(|| {
            self.err(format!(
                "`{key}` = {s:?}: expected \"0x\" and {digits} lowercase hex digits"
            ))
        })
    }
    /// Reads a `"0x"` string of 8 lowercase hex digits.
    pub fn hex_u32(&mut self, key: &str) -> Result<u32, NativeError> {
        self.hex(key, 8).map(|v| v as u32)
    }
    /// Reads a `"0x"` string of 4 lowercase hex digits.
    pub fn hex_u16(&mut self, key: &str) -> Result<u16, NativeError> {
        self.hex(key, 4).map(|v| v as u16)
    }
    /// Reads a `"0x"` string of 2 lowercase hex digits.
    pub fn hex_u8(&mut self, key: &str) -> Result<u8, NativeError> {
        self.hex(key, 2).map(|v| v as u8)
    }

    /// An array of integers.
    pub fn ints(&mut self, key: &str) -> Result<Vec<i64>, NativeError> {
        match self.take(key)? {
            toml::Value::Array(a) => a
                .into_iter()
                .map(|v| match v {
                    toml::Value::Integer(i) => Ok(i),
                    _ => Err(self.err(format!("`{key}` must hold integers"))),
                })
                .collect(),
            _ => Err(self.err(format!("`{key}` must be an array"))),
        }
    }

    /// An array of strings.
    pub fn strs(&mut self, key: &str) -> Result<Vec<String>, NativeError> {
        match self.take(key)? {
            toml::Value::Array(a) => a
                .into_iter()
                .map(|v| match v {
                    toml::Value::String(s) => Ok(s),
                    _ => Err(self.err(format!("`{key}` must hold strings"))),
                })
                .collect(),
            _ => Err(self.err(format!("`{key}` must be an array"))),
        }
    }

    /// A sub-table.
    pub fn table(&mut self, key: &str) -> Result<Fields, NativeError> {
        match self.take(key)? {
            toml::Value::Table(t) => Ok(self.child(key, t)),
            _ => Err(self.err(format!("`{key}` must be a table"))),
        }
    }

    /// An optional sub-table (e.g. `[encoding]`).
    pub fn opt_table(&mut self, key: &str) -> Result<Option<Fields>, NativeError> {
        if self.has(key) {
            self.table(key).map(Some)
        } else {
            Ok(None)
        }
    }

    /// An array of tables, in order.
    pub fn tables(&mut self, key: &str) -> Result<Vec<Fields>, NativeError> {
        if !self.has(key) {
            return Ok(Vec::new());
        }
        match self.take(key)? {
            toml::Value::Array(a) => a
                .into_iter()
                .enumerate()
                .map(|(i, v)| match v {
                    toml::Value::Table(t) => Ok(self.child(&format!("{key}[{i}]"), t)),
                    _ => Err(self.err(format!("`{key}` must hold tables"))),
                })
                .collect(),
            _ => Err(self.err(format!("`{key}` must be an array of tables"))),
        }
    }

    fn child(&self, name: &str, table: toml::Table) -> Fields {
        let ctx = if self.ctx.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", self.ctx)
        };
        Fields {
            file: self.file.clone(),
            ctx,
            table,
        }
    }

    /// The `native` / `native_version` header.
    pub fn header(&mut self, kind: &str, version: u32) -> Result<(), NativeError> {
        let k = self.str("native")?;
        if k != kind {
            return Err(self.err(format!("native = {k:?}, expected {kind:?}")));
        }
        let v = self.int("native_version")?;
        if v != i64::from(version) {
            return Err(self.err(format!(
                "native_version {v} unknown (this build: {version})"
            )));
        }
        Ok(())
    }

    /// Errors on any key not read (unknown input is an error).
    pub fn finish(&self) -> Result<(), NativeError> {
        if let Some(k) = self.table.keys().next() {
            return Err(self.err(format!("unknown key `{k}`")));
        }
        Ok(())
    }
}

//! Little-endian cursor over a byte slice, and the [`FormatError`] type, shared by the
//! file-format parsers in `d2-formats`.
//!
//! Every read is bounds-checked and returns `FormatError::Truncated` instead of panicking,
//! which is how the parsers reject short or damaged files. [`FormatError`] is re-exported
//! from the crate root; the cursor itself is crate-private.

/// A format parser rejected its input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    /// The data ended before a field could be read.
    #[error("{format}: data ends early (needed {needed} bytes at offset {offset:#x})")]
    Truncated {
        /// Name of the format being parsed.
        format: &'static str,
        /// Offset where the read started.
        offset: usize,
        /// Number of bytes the read needed.
        needed: usize,
    },
    /// The data was long enough but a field or structure is not valid.
    #[error("{format}: {message}")]
    Invalid {
        /// Name of the format being parsed.
        format: &'static str,
        /// What was wrong.
        message: String,
    },
}

/// A [`FormatError::Invalid`] for `format` with `message`.
pub(crate) fn invalid(format: &'static str, message: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format,
        message: message.into(),
    }
}

/// A read position in a file's bytes. `format` names the file format in the
/// errors it returns.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    format: &'static str,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `data`.
    pub(crate) fn new(data: &'a [u8], format: &'static str) -> Self {
        Self {
            data,
            pos: 0,
            format,
        }
    }

    /// A cursor at byte `pos` of `data` (reads fail if it is past the end).
    pub(crate) fn at(data: &'a [u8], pos: usize, format: &'static str) -> Self {
        Self { data, pos, format }
    }

    /// The current byte offset.
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    /// The next `n` bytes, or [`FormatError::Truncated`] if fewer remain.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            None => Err(FormatError::Truncated {
                format: self.format,
                offset: self.pos,
                needed: n,
            }),
        }
    }

    /// The next `N` bytes as an array.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], FormatError> {
        let b = self.bytes(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(b);
        Ok(a)
    }

    /// Reads a byte.
    pub(crate) fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.array::<1>()?[0])
    }

    /// Reads a little-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    /// Reads a little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Reads a little-endian `i32`.
    pub(crate) fn i32(&mut self) -> Result<i32, FormatError> {
        Ok(i32::from_le_bytes(self.array()?))
    }
}

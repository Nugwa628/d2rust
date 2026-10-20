//! The LSB-first bit reader every item record is read with.

use super::ItemBitsError;

/// LSB-first bit reader: bit 0 of byte 0 is the first bit of the stream,
/// and a value of n bits takes its bit 0 from the earliest stream bit.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// A reader at bit 0 of `buf`.
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bits 0..n−1 of the next n bits (n ≤ 32). Reading past the end is
    /// [`ItemBitsError::Short`] at the current position, and reads nothing.
    pub fn read(&mut self, n: u32) -> Result<u32, ItemBitsError> {
        let n = n as usize;
        if self.pos + n > self.buf.len() * 8 {
            return Err(ItemBitsError::Short(self.pos));
        }
        let mut v = 0u32;
        for i in 0..n {
            v |= u32::from(bit_set(self.buf, self.pos + i)) << i;
        }
        self.pos += n;
        Ok(v)
    }

    /// Bits read so far.
    pub fn pos(&self) -> usize {
        self.pos
    }
}

/// Whether stream bit `at` of `buf` is 1 (LSB-first).
pub(super) fn bit_set(buf: &[u8], at: usize) -> bool {
    (buf[at / 8] >> (at % 8)) & 1 != 0
}

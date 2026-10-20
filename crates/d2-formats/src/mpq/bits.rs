//! LSB-first bit reader and writer for the MPQ bit-stream codecs.
//!
//! The PKWARE DCL and Huffman streams inside archive sectors pack their
//! codes least significant bit first: the first code starts at bit 0 of the
//! first byte. [`BitReader`] feeds the decoders in `explode` and `huffman`;
//! [`build_decode_table`] turns a code/length table into a lookup table
//! indexed by the next bits of the stream. `BitWriter` is the matching
//! writer the unit tests use to build streams by hand.

/// Reads an LSB-first bit stream. `peek` zero-fills past the end of the
/// input, so table lookups near the end are safe; consuming bits that don't
/// exist is an error.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

/// The input ended in the middle of a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfBits;

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            count: 0,
        }
    }

    fn refill(&mut self) {
        while self.count <= 56 && self.pos < self.data.len() {
            self.buf |= u64::from(self.data[self.pos]) << self.count;
            self.pos += 1;
            self.count += 8;
        }
    }

    /// The next `n` (≤ 32) bits without consuming them.
    pub(crate) fn peek(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        self.refill();
        (self.buf & ((1u64 << n) - 1)) as u32
    }

    /// Drops the next `n` bits; an error if the input has fewer left.
    pub(crate) fn consume(&mut self, n: u32) -> Result<(), OutOfBits> {
        self.refill();
        if n > self.count {
            return Err(OutOfBits);
        }
        self.buf >>= n;
        self.count -= n;
        Ok(())
    }

    /// The next `n` (≤ 32) bits, consumed.
    pub(crate) fn read(&mut self, n: u32) -> Result<u32, OutOfBits> {
        let v = self.peek(n);
        self.consume(n)?;
        Ok(v)
    }
}

/// Builds a lookup table for an LSB-first prefix code: entry `j` holds the
/// symbol whose code matches the low bits of `j`. `N` must be
/// `1 << (longest code length)`.
pub(crate) const fn build_decode_table<const N: usize>(codes: &[u16], bits: &[u8]) -> [u8; N] {
    let mut table = [0u8; N];
    let mut sym = 0;
    while sym < codes.len() {
        let step = 1usize << bits[sym];
        let mut j = codes[sym] as usize;
        while j < N {
            table[j] = sym as u8;
            j += step;
        }
        sym += 1;
    }
    table
}

/// LSB-first bit writer, the inverse of [`BitReader`]. Used by tests to
/// build valid streams.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct BitWriter {
    pub(crate) bytes: Vec<u8>,
    bits: u32,
}

#[cfg(test)]
impl BitWriter {
    /// Appends the low `n` bits of `v`, least significant first.
    pub(crate) fn write(&mut self, v: u32, n: u32) {
        for k in 0..n {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if v >> k & 1 != 0 {
                *self.bytes.last_mut().expect("pushed") |= 1 << (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsb_first_order() {
        let mut r = BitReader::new(&[0b1010_0001, 0xFF]);
        assert_eq!(r.read(1), Ok(1));
        assert_eq!(r.read(4), Ok(0b0000));
        assert_eq!(r.read(3), Ok(0b101));
        assert_eq!(r.read(8), Ok(0xFF));
        assert_eq!(r.peek(8), 0, "zero-filled past the end");
        assert_eq!(r.read(1), Err(OutOfBits));
    }

    #[test]
    fn writer_round_trip() {
        let mut w = BitWriter::default();
        w.write(1, 1);
        w.write(0, 4);
        w.write(0b101, 3);
        w.write(0x1FF, 9);
        assert_eq!(w.bytes, [0b1010_0001, 0xFF, 0x01]);
    }
}

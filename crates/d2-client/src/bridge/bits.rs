//! The bit reader of the bit-packed S→C messages (0x18, 0x95, 0x96,
//! 0xAC, the item stream): bits from the lowest unread bit of the current
//! byte upward; a read past the end returns the bits that remain (high
//! bits 0) and sets the overflow flag.

/// A bit reader over a byte buffer.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Set by a read that ran past the end (+0x10).
    pub overflow: bool,
}

impl<'a> BitReader<'a> {
    /// Capacity `buf.len × 8` bits, position 0.
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            overflow: false,
        }
    }

    /// Bits read so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Reads `n` (≤ 32) unsigned bits.
    pub fn read(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        let cap = self.buf.len() * 8;
        let mut v = 0u32;
        for i in 0..n {
            if self.pos >= cap {
                self.overflow = true;
                break;
            }
            let bit = (self.buf[self.pos / 8] >> (self.pos % 8)) & 1;
            v |= u32::from(bit) << i;
            self.pos += 1;
        }
        v
    }

    /// Reads `n` bits, sign-extended when `n < 32` and bit `n − 1` is set.
    pub fn read_signed(&mut self, n: u32) -> i32 {
        let v = self.read(n);
        if n > 0 && n < 32 && v & (1 << (n - 1)) != 0 {
            (v | (u32::MAX << n)) as i32
        } else {
            v as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_bits_first_overflow_and_sign() {
        let mut r = BitReader::new(&[0b1010_1101, 0xFF]);
        assert_eq!(r.read(3), 0b101);
        assert_eq!(r.read(5), 0b10101);
        assert_eq!(r.position(), 8);
        assert!(!r.overflow);
        // 4 bits 1111 signed → −1.
        assert_eq!(r.read_signed(4), -1);
        // 6 bits wanted, 4 remain: the remaining bits, high bits 0.
        assert_eq!(r.read(6), 0b1111);
        assert!(r.overflow);
        let mut r = BitReader::new(&[0x07]);
        assert_eq!(r.read_signed(4), 7);
        assert_eq!(BitReader::new(&[0xFF; 4]).read_signed(32), -1);
        assert_eq!(BitReader::new(&[]).read(0), 0);
    }
}

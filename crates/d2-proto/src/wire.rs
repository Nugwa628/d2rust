//! Explicit little-endian reads and writes for the typed messages of
//! [`crate::generated`].
//!
//! Every multi-byte value on the wire is little-endian and sits at a fixed
//! byte offset counted from the id byte (offset 0). The generated
//! decode/encode code calls these helpers instead of casting structs, so the
//! byte layout never depends on host endianness or struct padding.
//!
//! Writing is additive: [`start`] zeroes the buffer, then every field ORs its
//! bytes in. Fields of one layout never share a bit, so the order the fields
//! are written in does not matter, and bit fields packed into one u32 (see
//! [`put_bits`] / [`put_bit`]) combine without masking.

/// A typed decode that failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The input has no bytes, so there is no id to read.
    #[error("message is empty")]
    Empty,
    /// The first byte is not the id of the message type being decoded.
    #[error("id 0x{found:02X}, expected 0x{expected:02X}")]
    WrongId { expected: u8, found: u8 },
    /// The input length differs from the message's fixed size.
    #[error("{found} bytes, expected {expected}")]
    WrongSize { expected: usize, found: usize },
}

/// A message with a fixed size and a layout of fixed fields. Bytes the
/// layout does not list (no field covers them) decode to nothing and encode
/// as 0.
pub trait FixedMessage: Sized {
    /// First byte of every message of this type.
    const ID: u8;
    /// Total length in bytes, id included.
    const SIZE: usize;
    /// Decodes exactly `SIZE` bytes starting with `ID`.
    fn decode(b: &[u8]) -> Result<Self, DecodeError>;
    /// Writes the message into `out` (`SIZE` bytes, zeroed first).
    /// Panics if `out` has another length or a bit field holds a value
    /// wider than its bits.
    fn write(&self, out: &mut [u8]);
}

// ---- decode

/// The checks every fixed decode starts with, in this order: an id byte
/// exists, it is `id`, and the input is exactly `size` bytes.
pub(crate) fn check(b: &[u8], id: u8, size: usize) -> Result<(), DecodeError> {
    match b.first() {
        None => Err(DecodeError::Empty),
        Some(&f) if f != id => Err(DecodeError::WrongId {
            expected: id,
            found: f,
        }),
        _ if b.len() != size => Err(DecodeError::WrongSize {
            expected: size,
            found: b.len(),
        }),
        _ => Ok(()),
    }
}

/// The byte at `off`.
pub(crate) fn u8_at(b: &[u8], off: usize) -> u8 {
    b[off]
}

/// The little-endian u16 at `off`.
pub(crate) fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(bytes_at(b, off))
}

/// The little-endian u32 at `off`.
pub(crate) fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes_at(b, off))
}

/// The 16 bytes at `off` (a zero-padded name field).
pub(crate) fn bytes16_at(b: &[u8], off: usize) -> [u8; 16] {
    bytes_at(b, off)
}

/// The `N` bytes at `off`, as they are.
pub(crate) fn bytes_at<const N: usize>(b: &[u8], off: usize) -> [u8; N] {
    let mut a = [0; N];
    a.copy_from_slice(&b[off..off + N]);
    a
}

/// Bits `0..n` of the u32 at `off` (a bit field packed at the low end).
pub(crate) fn bits_at(b: &[u8], off: usize, n: u32) -> u32 {
    u32_at(b, off) & ((1u32 << n) - 1)
}

/// Bit `n` of the u32 at `off` (a flag).
pub(crate) fn bit_at(b: &[u8], off: usize, n: u32) -> bool {
    u32_at(b, off) >> n & 1 == 1
}

// ---- encode

/// Zeroes `out` and writes the id. Panics unless `out` is `size` bytes.
pub(crate) fn start(out: &mut [u8], id: u8, size: usize) {
    assert_eq!(out.len(), size, "output buffer size");
    out.fill(0);
    out[0] = id;
}

/// ORs `v`'s bytes in at `off`.
fn or_bytes(out: &mut [u8], off: usize, v: &[u8]) {
    for (o, b) in out[off..off + v.len()].iter_mut().zip(v) {
        *o |= b;
    }
}

/// Writes the byte `v` at `off`.
pub(crate) fn put_u8(out: &mut [u8], off: usize, v: u8) {
    or_bytes(out, off, &[v]);
}

/// Writes `v` little-endian at `off`.
pub(crate) fn put_u16(out: &mut [u8], off: usize, v: u16) {
    or_bytes(out, off, &v.to_le_bytes());
}

/// Writes `v` little-endian at `off`.
pub(crate) fn put_u32(out: &mut [u8], off: usize, v: u32) {
    or_bytes(out, off, &v.to_le_bytes());
}

/// Writes 16 bytes at `off` (a zero-padded name field).
pub(crate) fn put_bytes16(out: &mut [u8], off: usize, v: &[u8; 16]) {
    or_bytes(out, off, v);
}

/// Writes `N` bytes at `off`, as they are.
pub(crate) fn put_bytes<const N: usize>(out: &mut [u8], off: usize, v: &[u8; N]) {
    or_bytes(out, off, v);
}

/// Writes `v` into bits `0..n` of the u32 at `off`. Panics if `v` needs
/// more than `n` bits.
pub(crate) fn put_bits(out: &mut [u8], off: usize, n: u32, v: u32) {
    assert!(v < 1u32 << n, "value {v} does not fit in {n} bits");
    put_u32(out, off, v);
}

/// Sets bit `n` of the u32 at `off` when `v` is true.
pub(crate) fn put_bit(out: &mut [u8], off: usize, n: u32, v: bool) {
    put_u32(out, off, (v as u32) << n);
}

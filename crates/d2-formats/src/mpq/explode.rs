//! PKWARE Data Compression Library (DCL) decoder for MPQ sectors.
//!
//! [`explode`] decodes the stream stored under the IMPLODE block flag or the PKWARE
//! compression mask bit. The two header bytes give literal mode (binary or ASCII) and
//! dictionary size; the rest is LSB-first literals and length/distance copies, ending with
//! an end marker. `Archive` calls it from the sector decompressor; the code tables live in
//! `tables`.

use super::bits::{build_decode_table, BitReader, OutOfBits};
use super::tables::{
    CH_BITS, CH_CODE, DIST_BITS, DIST_CODE, EX_LEN_BITS, LEN_BASE, LEN_BITS, LEN_CODE,
};
use super::CodecError;

const LEN_DECODE: [u8; 256] = build_decode_table(&LEN_CODE, &LEN_BITS);
const DIST_DECODE: [u8; 256] = build_decode_table(&DIST_CODE, &DIST_BITS);
const CH_DECODE: [u8; 1 << 13] = build_decode_table(&CH_CODE, &CH_BITS);

/// Length value that marks the end of the stream.
const END_MARKER: u32 = 0x205;

/// Output bytes per input byte, at most: the densest code is a 518-byte
/// copy in 22 bits (under 190 bytes per input byte). Bounds the
/// preallocation, since `max_out` is untrusted.
const MAX_RATIO: usize = 256;

fn err(reason: &'static str) -> CodecError {
    CodecError {
        codec: "pkware",
        reason,
    }
}

impl From<OutOfBits> for CodecError {
    fn from(_: OutOfBits) -> Self {
        CodecError {
            codec: "bitstream",
            reason: "input ended in the middle of a code",
        }
    }
}

/// Decompresses a PKWARE DCL stream. Output is capped at `max_out` bytes;
/// the caller checks the final length.
pub(crate) fn explode(input: &[u8], max_out: usize) -> Result<Vec<u8>, CodecError> {
    if input.len() <= 4 {
        return Err(err("input too short"));
    }
    let ascii = match input[0] {
        0 => false,
        1 => true,
        _ => return Err(err("invalid literal mode")),
    };
    let dict_bits = u32::from(input[1]);
    if !(4..=6).contains(&dict_bits) {
        return Err(err("invalid dictionary size"));
    }

    let mut r = BitReader::new(&input[2..]);
    let mut out = Vec::with_capacity(max_out.min(input.len().saturating_mul(MAX_RATIO)));
    // Each item starts with one flag bit: 1 for a copy from earlier output, 0 for a
    // literal byte (8 raw bits in binary mode, a prefix code in ASCII mode).
    while out.len() < max_out {
        if r.read(1)? == 1 {
            let sym = usize::from(LEN_DECODE[r.peek(8) as usize]);
            r.consume(u32::from(LEN_BITS[sym]))?;
            let mut len = sym as u32;
            let extra = u32::from(EX_LEN_BITS[sym]);
            if extra > 0 {
                len = u32::from(LEN_BASE[sym]) + r.read(extra)?;
            }
            if len == END_MARKER {
                break;
            }
            let n = len as usize + 2;

            // The distance is a coded high part plus raw low bits: 2 bits for a
            // 2-byte copy (which only reaches nearby bytes), the dictionary size's
            // bits for longer ones.
            let dsym = u32::from(DIST_DECODE[r.peek(8) as usize]);
            r.consume(u32::from(DIST_BITS[dsym as usize]))?;
            let dist = if n == 2 {
                (dsym << 2) | r.read(2)?
            } else {
                (dsym << dict_bits) | r.read(dict_bits)?
            };
            let back = dist as usize + 1;
            if back > out.len() {
                return Err(err("copy distance before start of output"));
            }
            // Byte by byte: a copy may overlap the bytes it is producing.
            for _ in 0..n.min(max_out - out.len()) {
                out.push(out[out.len() - back]);
            }
        } else {
            let byte = if ascii {
                let sym = CH_DECODE[r.peek(13) as usize];
                r.consume(u32::from(CH_BITS[usize::from(sym)]))?;
                sym
            } else {
                r.read(8)? as u8
            };
            out.push(byte);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_test_vector() {
        let out = explode(&[0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F], 64).unwrap();
        assert_eq!(out, b"AIAIAIAIAIAIA");
    }

    #[test]
    fn output_is_capped() {
        let out = explode(&[0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F], 5).unwrap();
        assert_eq!(out, b"AIAIA");
    }

    #[test]
    fn rejects_bad_headers() {
        assert!(explode(&[0x02, 0x04, 0, 0, 0], 8).is_err(), "mode");
        assert!(explode(&[0x00, 0x07, 0, 0, 0], 8).is_err(), "dict size");
        assert!(explode(&[0x00, 0x04, 0, 0], 8).is_err(), "too short");
    }

    #[test]
    fn truncated_stream_is_an_error() {
        assert!(explode(&[0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80], 64).is_err());
    }

    #[test]
    fn ascii_codes_are_a_prefix_code() {
        // Every 13-bit pattern must decode to a symbol whose code matches it.
        for (j, &sym) in CH_DECODE.iter().enumerate() {
            let sym = usize::from(sym);
            let mask = (1usize << CH_BITS[sym]) - 1;
            assert_eq!(j & mask, usize::from(CH_CODE[sym]), "pattern {j:#x}");
        }
    }
}

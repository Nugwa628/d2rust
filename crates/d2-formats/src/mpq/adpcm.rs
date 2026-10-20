//! The MPQ flavour of IMA ADPCM, for sector audio.
//!
//! [`decompress`] turns the ADPCM-compressed sectors of `.wav` files into 16-bit little-endian
//! samples, mono or stereo. `Archive` calls it from the sector decompressor when the ADPCM
//! mask bits are set; the step tables are in `tables`.
//!
//! A stream is two header bytes (the second is the `shift` that sets each step's base
//! rounding term), one starting sample per channel, then one byte per op. Channels take
//! turns op by op. A data op (top bit clear) moves the channel's sample by a step: bits
//! 0–5 pick which halvings of the current step size add up, bit 6 is the sign, and the
//! step index then moves by the change table. A control op (top bit set) repeats the
//! sample, nudges the step index by 8, or skips to the other channel.

use super::tables::{ADPCM_CHANGE_TABLE, ADPCM_STEP_SIZE};

/// Step index every channel starts at.
const INITIAL_INDEX: i32 = 44;
/// Highest step index (the step size table has 89 entries).
const MAX_INDEX: i32 = 88;

/// Control op: emit the channel's sample again and step the index down by one.
const OP_REPEAT: u8 = 0;
/// Control op: step the index up by 8.
const OP_INDEX_UP: u8 = 1;
/// Control op: move on to the next channel without output.
const OP_NEXT_CHANNEL: u8 = 2;
/// Bit 6 of a data op: subtract the step instead of adding it.
const SIGN_BIT: u8 = 0x40;

/// Output bytes per input byte, at most: each input byte yields at most one
/// 2-byte sample. Bounds the preallocation, since `max_out` is untrusted.
const MAX_RATIO: usize = 2;

/// Decodes `channels` (1 or 2) interleaved ADPCM channels into i16 LE
/// samples, at most `max_out` bytes. Malformed input just ends the output
/// early; the caller checks the final length.
pub(crate) fn decompress(input: &[u8], channels: usize, max_out: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(max_out.min(input.len().saturating_mul(MAX_RATIO)));
    if input.len() < 2 {
        return out;
    }
    let shift = u32::from(input[1]);
    let mut pos = 2;

    let mut sample = [0i32; 2];
    let mut index = [INITIAL_INDEX; 2];
    for s in sample.iter_mut().take(channels) {
        if pos + 2 > input.len() || out.len() + 2 > max_out {
            return out;
        }
        *s = i32::from(i16::from_le_bytes([input[pos], input[pos + 1]]));
        pos += 2;
        out.extend_from_slice(&(*s as i16).to_le_bytes());
    }

    let mut ch = 0;
    for &op in &input[pos..] {
        if out.len() + 2 > max_out {
            break;
        }
        if op & 0x80 != 0 {
            match op & 0x7F {
                OP_REPEAT => {
                    if index[ch] > 0 {
                        index[ch] -= 1;
                    }
                    out.extend_from_slice(&(sample[ch] as i16).to_le_bytes());
                    ch = (ch + 1) % channels;
                }
                OP_INDEX_UP => index[ch] = (index[ch] + 8).min(MAX_INDEX),
                OP_NEXT_CHANNEL => ch = (ch + 1) % channels,
                // Every other control op steps the index down by 8.
                _ => index[ch] = (index[ch] - 8).max(0),
            }
        } else {
            let base = ADPCM_STEP_SIZE[index[ch] as usize];
            let mut diff = base >> shift.min(31);
            for k in 0..6 {
                if op & (1 << k) != 0 {
                    diff += base >> k;
                }
            }
            sample[ch] = if op & SIGN_BIT != 0 {
                (sample[ch] - diff).max(-32768)
            } else {
                (sample[ch] + diff).min(32767)
            };
            out.extend_from_slice(&(sample[ch] as i16).to_le_bytes());
            index[ch] =
                (index[ch] + ADPCM_CHANGE_TABLE[usize::from(op & 0x1F)]).clamp(0, MAX_INDEX);
            ch = (ch + 1) % channels;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    #[test]
    fn encoded_sample() {
        let out = decompress(&[0x00, 0x00, 0x10, 0x00, 0x01], 1, 64);
        assert_eq!(samples(&out), [16, 1004]);
    }

    #[test]
    fn repeat_command() {
        let out = decompress(&[0x00, 0x00, 0x10, 0x00, 0x80], 1, 64);
        assert_eq!(samples(&out), [16, 16]);
    }

    #[test]
    fn negative_sample_clamps() {
        // Initial -32000, then subtract a large step: clamps at -32768.
        let init = (-32000i16).to_le_bytes();
        let out = decompress(&[0x00, 0x00, init[0], init[1], 0x7F], 1, 64);
        assert_eq!(samples(&out), [-32000, -32768]);
    }

    #[test]
    fn stereo_interleaves() {
        // Two initial samples, then a repeat on each channel.
        let out = decompress(&[0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x80, 0x80], 2, 64);
        assert_eq!(samples(&out), [1, 2, 1, 2]);
    }

    #[test]
    fn output_is_capped() {
        let out = decompress(&[0x00, 0x00, 0x10, 0x00, 0x80, 0x80, 0x80], 1, 4);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn header_bytes() {
        // Fewer than 2 bytes: no output.
        assert!(decompress(&[], 1, 64).is_empty());
        assert!(decompress(&[0x05], 2, 64).is_empty());
        // Byte 0 is ignored.
        let body = [0x10, 0x00, 0x01, 0x45, 0x80];
        let base: Vec<u8> = [0x00, 0x03].iter().chain(&body).copied().collect();
        for b0 in [0x01, 0x7F, 0xFF] {
            let mut other = base.clone();
            other[0] = b0;
            assert_eq!(decompress(&other, 1, 64), decompress(&base, 1, 64));
        }
        // Byte 1 is `shift`: diff = (base >> shift) + (base >> 0) for op 1,
        // base = StepSize[44] = 0x1EE.
        for (shift, want) in [(0u8, 16 + 0x1EE + 0x1EE), (3, 16 + (0x1EE >> 3) + 0x1EE)] {
            let out = decompress(&[0x00, shift, 0x10, 0x00, 0x01], 1, 64);
            assert_eq!(samples(&out), [16, want as i16]);
        }
    }
}

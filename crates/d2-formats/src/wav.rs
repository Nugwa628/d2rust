//! RIFF/WAVE sound files from the game's sound archives: a chunk walk that does not skip
//! RIFF pad bytes, and the `data` bytes read as interleaved little-endian i16 samples.
//!
//! [`Wav::parse`] takes the decoded file bytes and returns every format field plus the
//! samples; the audio layer decides what to play. Whether a sound is playable (tag 1,
//! 16-bit, 22,050 Hz, mono or stereo) is checked separately, see [`Wav::is_playable`].

use crate::cursor::{invalid, FormatError};

const FORMAT: &str = "wav";

/// The only sample rate the audio layer plays, in Hz.
pub const PLAYABLE_RATE: u32 = 22_050;
/// The only bits-per-sample value the audio layer plays.
pub const PLAYABLE_BITS: u16 = 16;
/// PCM format tag, the only one the audio layer plays.
pub const FORMAT_PCM: u16 = 1;

/// A parsed `.wav`: the `fmt ` fields and the decoded samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav {
    /// Format tag from `fmt ` (1 is PCM).
    pub format_tag: u16,
    /// Channel count.
    pub channels: u16,
    /// Sample rate in Hz.
    pub rate: u32,
    /// Average bytes per second, as stored.
    pub byte_rate: u32,
    /// Bits per sample.
    pub bits: u16,
    /// Bytes per sample frame, as stored.
    pub block_align: u16,
    /// `data` bytes as i16 LE, interleaved by channel (L, R for stereo).
    /// A trailing odd byte is dropped.
    pub samples: Vec<i16>,
}

impl Wav {
    /// Parses a file's bytes after MPQ decoding.
    pub fn parse(file: &[u8]) -> Result<Wav, FormatError> {
        // At least 32 bytes, RIFF at 0, WAVE at 8; the RIFF size field is not read
        // (`riff_size_field_is_not_read`).
        if file.len() < 32 {
            return Err(invalid(
                FORMAT,
                format!("file is {} bytes, needs at least 32", file.len()),
            ));
        }
        if &file[0..4] != b"RIFF" || &file[8..12] != b"WAVE" {
            return Err(invalid(FORMAT, "no RIFF/WAVE header"));
        }
        let mut remaining = file.len() - 12;
        // First the `fmt ` chunk is searched for, starting at offset 12.
        let (fmt_at, fmt_size) = find_chunk(file, 12, &mut remaining, b"fmt ")
            .ok_or_else(|| invalid(FORMAT, "no `fmt ` chunk"))?;
        if fmt_size < 16 || fmt_size > remaining {
            return Err(invalid(
                FORMAT,
                format!("`fmt ` size {fmt_size} (remaining {remaining})"),
            ));
        }
        remaining -= fmt_size;
        let f = &file[fmt_at..fmt_at + 16];
        let u16_at = |o: usize| u16::from_le_bytes([f[o], f[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]);
        // Then `data` is searched for, starting right after the `fmt ` body.
        let (data_at, data_size) = find_chunk(file, fmt_at + fmt_size, &mut remaining, b"data")
            .ok_or_else(|| invalid(FORMAT, "no `data` chunk after `fmt `"))?;
        if data_size > remaining {
            return Err(invalid(
                FORMAT,
                format!("`data` size {data_size} runs past the file end (remaining {remaining})"),
            ));
        }
        // The body read as i16 LE.
        let samples = file[data_at..data_at + data_size]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b))
            .collect();
        Ok(Wav {
            format_tag: u16_at(0),
            channels: u16_at(2),
            rate: u32_at(4),
            byte_rate: u32_at(8),
            block_align: u16_at(12),
            bits: u16_at(14),
            samples,
        })
    }

    /// Frames = samples / channels (0 channels gives 0).
    pub fn frames(&self) -> usize {
        match self.channels {
            0 => 0,
            c => self.samples.len() / c as usize,
        }
    }

    /// The audio layer's acceptance check: tag 1, 16-bit, 22,050 Hz, one or two
    /// channels. Files that fail it are treated as failed to load.
    pub fn is_playable(&self) -> bool {
        self.format_tag == FORMAT_PCM
            && self.bits == PLAYABLE_BITS
            && self.rate == PLAYABLE_RATE
            && matches!(self.channels, 1 | 2)
    }
}

/// The chunk search. Starts at `at` with the
/// caller's `remaining`; on success returns (body offset, size) and leaves
/// `remaining` counting the body. Pad bytes after odd chunks are not
/// skipped.
fn find_chunk(
    file: &[u8],
    mut at: usize,
    remaining: &mut usize,
    id: &[u8; 4],
) -> Option<(usize, usize)> {
    loop {
        if *remaining < 8 {
            return None;
        }
        *remaining -= 8;
        let h = &file[at..at + 8];
        let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let body = at + 8;
        if &h[0..4] == id {
            return Some((body, size));
        }
        if *remaining < size {
            return None;
        }
        *remaining -= size;
        at = body + size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    fn fmt16(channels: u16) -> Vec<u8> {
        let align = 2 * channels;
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&22_050u32.to_le_bytes());
        b.extend_from_slice(&(22_050 * align as u32).to_le_bytes());
        b.extend_from_slice(&align.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        chunk(b"fmt ", &b)
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(&body);
        v
    }

    fn base() -> Vec<u8> {
        riff(&[fmt16(1), chunk(b"data", &[0x01, 0x00, 0xFF, 0xFF])])
    }

    #[test]
    fn mono_48_bytes() {
        let f = base();
        assert_eq!(f.len(), 48);
        assert_eq!(&f[4..8], &40u32.to_le_bytes());
        let w = Wav::parse(&f).unwrap();
        assert_eq!(
            (w.channels, w.rate, w.bits, w.format_tag),
            (1, 22_050, 16, 1)
        );
        assert_eq!((w.byte_rate, w.block_align), (44_100, 2));
        assert_eq!(w.samples, vec![1, -1]);
        assert_eq!(w.frames(), 2);
        assert!(w.is_playable());
    }

    #[test]
    fn riff_size_field_is_not_read() {
        let mut f = base();
        f[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Wav::parse(&f).unwrap().samples, vec![1, -1]);
    }

    #[test]
    fn pad_chunk_is_skipped() {
        let f = riff(&[
            fmt16(1),
            chunk(b"PAD ", &[0; 4]),
            chunk(b"data", &[1, 0, 0xFF, 0xFF]),
        ]);
        assert_eq!(Wav::parse(&f).unwrap().samples, vec![1, -1]);
    }

    #[test]
    fn odd_chunk_misaligns_the_walk() {
        let mut list = chunk(b"LIST", &[0xAA; 3]);
        list.push(0); // RIFF pad byte, not skipped by the parser
        let f = riff(&[fmt16(1), list, chunk(b"data", &[1, 0, 0xFF, 0xFF])]);
        assert_eq!(f.len(), 60);
        let e = Wav::parse(&f).unwrap_err();
        assert!(matches!(e, FormatError::Invalid { .. }), "{e}");
        // The misread header is 00 'd' 'a' 't' with size 0x461.
        assert_eq!(&f[47..51], b"\0dat");
        assert_eq!(u32::from_le_bytes(f[51..55].try_into().unwrap()), 0x461);
    }

    #[test]
    fn data_before_fmt_fails() {
        let f = riff(&[chunk(b"data", &[1, 0, 0xFF, 0xFF]), fmt16(1)]);
        assert!(Wav::parse(&f).is_err());
    }

    #[test]
    fn truncated_data_fails() {
        let mut f = riff(&[fmt16(1)]);
        f.extend_from_slice(b"data");
        f.extend_from_slice(&6u32.to_le_bytes());
        f.extend_from_slice(&[1, 0, 0xFF, 0xFF]);
        assert!(Wav::parse(&f).is_err());
    }

    #[test]
    fn short_file_fails() {
        assert!(Wav::parse(&base()[..31]).is_err());
        assert!(Wav::parse(&[]).is_err());
        let mut f = base();
        f[8] = b'X';
        assert!(Wav::parse(&f).is_err());
    }

    #[test]
    fn stereo_interleaved() {
        let f = riff(&[fmt16(2), chunk(b"data", &[1, 0, 2, 0, 3, 0, 4, 0])]);
        let w = Wav::parse(&f).unwrap();
        assert_eq!((w.channels, w.byte_rate, w.block_align), (2, 88_200, 4));
        assert_eq!(w.frames(), 2);
        assert_eq!(w.samples, vec![1, 2, 3, 4]);
    }

    #[test]
    fn fmt_extra_bytes_skipped_and_short_fmt_fails() {
        let mut fmt = fmt16(1);
        fmt[4..8].copy_from_slice(&18u32.to_le_bytes());
        fmt.extend_from_slice(&[0x22, 0x00]); // cbSize, ignored
        let f = riff(&[fmt, chunk(b"data", &[1, 0, 0xFF, 0xFF])]);
        assert_eq!(Wav::parse(&f).unwrap().samples, vec![1, -1]);

        let mut fmt = fmt16(1);
        fmt[4..8].copy_from_slice(&14u32.to_le_bytes());
        fmt.truncate(8 + 14);
        let f = riff(&[fmt, chunk(b"data", &[1, 0, 0xFF, 0xFF, 0, 0])]);
        assert!(Wav::parse(&f).is_err());
    }

    #[test]
    fn odd_trailing_byte_dropped_and_format_check() {
        let mut fmt = fmt16(1);
        fmt[8 + 4..8 + 8].copy_from_slice(&11_025u32.to_le_bytes());
        let f = riff(&[fmt, chunk(b"data", &[1, 0, 2])]);
        let w = Wav::parse(&f).unwrap();
        assert_eq!(w.samples, vec![1]);
        assert!(!w.is_playable(), "11,025 Hz is rejected at sound start");
    }
}

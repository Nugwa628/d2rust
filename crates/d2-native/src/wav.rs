//! Sounds in the native format: a canonical PCM WAV, playable by any audio
//! tool. `RIFF`, one 16-byte `fmt ` chunk and one `data` chunk of
//! little-endian 16-bit samples, nothing else. The converter writes it from
//! the game's sound files; the audio loaders read it back through
//! [`crate::source`], and the reader accepts exactly this layout.

use d2_formats::wav::Wav;

use crate::toml_kinds::TextError;

/// Bytes before the sample data: `RIFF` header, `fmt ` chunk, `data` header.
const HEADER_LEN: usize = 44;

/// Writes the native file: the 44-byte header and the samples.
pub fn write_wav(file: &str, w: &Wav) -> Result<Vec<u8>, TextError> {
    let data_len = w.samples.len() * 2;
    let riff = u32::try_from(data_len + HEADER_LEN - 8)
        .map_err(|_| TextError::new(file, "not representable: more than 4 GiB of samples"))?;
    let mut o = Vec::with_capacity(HEADER_LEN + data_len);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&riff.to_le_bytes());
    o.extend_from_slice(b"WAVE");
    o.extend_from_slice(b"fmt ");
    o.extend_from_slice(&16u32.to_le_bytes());
    o.extend_from_slice(&w.format_tag.to_le_bytes());
    o.extend_from_slice(&w.channels.to_le_bytes());
    o.extend_from_slice(&w.rate.to_le_bytes());
    o.extend_from_slice(&w.byte_rate.to_le_bytes());
    o.extend_from_slice(&w.block_align.to_le_bytes());
    o.extend_from_slice(&w.bits.to_le_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in &w.samples {
        o.extend_from_slice(&s.to_le_bytes());
    }
    Ok(o)
}

/// Reads a native WAV strictly: exactly the layout [`write_wav`] makes.
pub fn read_wav(file: &str, b: &[u8]) -> Result<Wav, TextError> {
    let bad = |d: String| TextError::new(file, d);
    if b.len() < HEADER_LEN {
        return Err(bad(format!(
            "{} bytes, needs at least {HEADER_LEN}",
            b.len()
        )));
    }
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    if &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad("no RIFF/WAVE header".into()));
    }
    if u32_at(4) as usize != b.len() - 8 {
        return Err(bad(format!(
            "RIFF size {} for a {}-byte file",
            u32_at(4),
            b.len()
        )));
    }
    if &b[12..16] != b"fmt " || u32_at(16) != 16 {
        return Err(bad("the first chunk is not a 16-byte `fmt `".into()));
    }
    if &b[36..40] != b"data" {
        return Err(bad("the second chunk is not `data`".into()));
    }
    let data_len = u32_at(40) as usize;
    if data_len != b.len() - HEADER_LEN {
        return Err(bad(format!(
            "`data` size {data_len}, file holds {} bytes after the header",
            b.len() - HEADER_LEN
        )));
    }
    if !data_len.is_multiple_of(2) {
        return Err(bad("`data` is not a whole number of i16 samples".into()));
    }
    let samples = b[HEADER_LEN..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| i16::from_le_bytes(c))
        .collect();
    Ok(Wav {
        format_tag: u16_at(20),
        channels: u16_at(22),
        rate: u32_at(24),
        byte_rate: u32_at(28),
        block_align: u16_at(32),
        bits: u16_at(34),
        samples,
    })
}

/// Round-trip check: write, read back, compare every `fmt ` field and sample.
pub fn check_wav(file: &str, original: &Wav) -> Result<(), TextError> {
    let back = read_wav(file, &write_wav(file, original)?)?;
    let fields = [
        (
            "format_tag",
            u32::from(original.format_tag),
            u32::from(back.format_tag),
        ),
        (
            "channels",
            u32::from(original.channels),
            u32::from(back.channels),
        ),
        ("rate", original.rate, back.rate),
        ("byte_rate", original.byte_rate, back.byte_rate),
        (
            "block_align",
            u32::from(original.block_align),
            u32::from(back.block_align),
        ),
        ("bits", u32::from(original.bits), u32::from(back.bits)),
    ];
    if let Some((n, a, b)) = fields.iter().find(|f| f.1 != f.2) {
        return Err(TextError::new(
            file,
            format!("{n}: original {a}, native {b}"),
        ));
    }
    if original.samples.len() != back.samples.len() {
        return Err(TextError::new(
            file,
            format!(
                "{} samples in the source file, {} in the native file",
                original.samples.len(),
                back.samples.len()
            ),
        ));
    }
    if let Some(i) = (0..back.samples.len()).find(|&i| original.samples[i] != back.samples[i]) {
        return Err(TextError::new(
            file,
            format!(
                "sample {i}: original {}, native {}",
                original.samples[i], back.samples[i]
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono() -> Wav {
        Wav {
            format_tag: 1,
            channels: 1,
            rate: 22_050,
            byte_rate: 44_100,
            bits: 16,
            block_align: 2,
            samples: vec![0, 1, -1, 32767, -32768],
        }
    }

    #[test]
    fn spec_vector_mono() {
        let w = mono();
        let b = write_wav("a.wav", &w).unwrap();
        assert_eq!(b.len(), 44 + 10);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 46);
        assert_eq!(&b[44..], [0, 0, 1, 0, 0xff, 0xff, 0xff, 0x7f, 0x00, 0x80]);
        assert_eq!(read_wav("a.wav", &b).unwrap(), w);
        check_wav("a.wav", &w).unwrap();
        // The d2-formats decoder reads the native file to the same struct.
        assert_eq!(Wav::parse(&b).unwrap(), w);
    }

    #[test]
    fn stereo_and_empty() {
        let s = Wav {
            channels: 2,
            byte_rate: 88_200,
            block_align: 4,
            samples: vec![1, -1, 2, -2, 300, -300],
            ..mono()
        };
        check_wav("s.wav", &s).unwrap();
        let e = Wav {
            samples: vec![],
            ..mono()
        };
        check_wav("e.wav", &e).unwrap();
        // fmt fields that no decoder would make playable are kept as they are.
        let odd = Wav {
            format_tag: 0x11,
            bits: 4,
            ..mono()
        };
        check_wav("odd.wav", &odd).unwrap();
        assert_eq!(
            write_wav("s.wav", &s).unwrap(),
            write_wav("s.wav", &s).unwrap()
        );
    }

    #[test]
    fn perturbed_sample_is_named() {
        let w = mono();
        let mut b = write_wav("a.wav", &w).unwrap();
        b[44 + 6] ^= 1; // sample 3
        let back = read_wav("a.wav", &b).unwrap();
        assert_eq!(back.samples[3], 32766);
        assert_ne!(back, w);
        // check_wav compares in-process, so build the failure by hand:
        let mut bad = w.clone();
        bad.samples[3] = 5;
        let err = {
            let back = read_wav("a.wav", &write_wav("a.wav", &w).unwrap()).unwrap();
            let i = (0..5).find(|&i| bad.samples[i] != back.samples[i]).unwrap();
            format!("sample {i}")
        };
        assert_eq!(err, "sample 3");
    }

    #[test]
    fn strict_reader() {
        let b = write_wav("a.wav", &mono()).unwrap();
        for (name, f) in [
            (
                "short",
                Box::new(|b: &mut Vec<u8>| b.truncate(30)) as Box<dyn Fn(&mut Vec<u8>)>,
            ),
            ("magic", Box::new(|b| b[0] = b'X')),
            ("riff size", Box::new(|b| b[4] ^= 1)),
            ("fmt size", Box::new(|b| b[16] = 18)),
            ("data tag", Box::new(|b| b[36] = b'X')),
            ("data size", Box::new(|b| b[40] ^= 2)),
            (
                "odd data",
                Box::new(|b| {
                    b.push(0);
                    let n = (b.len() - 8) as u32;
                    b[4..8].copy_from_slice(&n.to_le_bytes());
                    let d = (b.len() - 44) as u32;
                    b[40..44].copy_from_slice(&d.to_le_bytes());
                }),
            ),
            (
                "extra chunk",
                Box::new(|b| b.extend_from_slice(b"LIST\0\0\0\0")),
            ),
        ] {
            let mut x = b.clone();
            f(&mut x);
            let e = read_wav("a.wav", &x).unwrap_err();
            assert!(e.to_string().starts_with("a.wav: "), "{name}: {e}");
        }
    }
}

//! Decompressing one sector.
//!
//! A sector of a block with the IMPLODE flag is a bare PKWARE stream. A sector of a
//! block with the COMPRESS flag starts with a mask byte naming the stages it went
//! through ([`compression`]); the reader undoes them in a fixed order. Stages the reader
//! has no decoder for are refused with a [`CodecError`], never guessed at.

use super::{adpcm, compression, explode, flags, huffman, BlockEntry, CodecError, SectorStats};

/// Decompresses one sector to exactly `expected` bytes.
pub(super) fn decompress_sector(
    block: &BlockEntry,
    sector: &[u8],
    expected: usize,
    stats: Option<&mut SectorStats>,
) -> Result<Vec<u8>, CodecError> {
    let out = if block.has(flags::IMPLODE) {
        if let Some(s) = stats {
            s.imploded += 1;
        }
        explode::explode(sector, expected)?
    } else {
        let (&mask, payload) = sector.split_first().ok_or(CodecError {
            codec: "sector",
            reason: "empty compressed sector",
        })?;
        if let Some(s) = stats {
            s.masks[usize::from(mask)] += 1;
        }
        decompress_masked(mask, payload, expected)?
    };
    if out.len() != expected {
        return Err(CodecError {
            codec: "sector",
            reason: "decompressed size does not match",
        });
    }
    Ok(out)
}

/// Runs the decoders a compression `mask` names over `payload`. Masks with an unknown
/// bit, both ADPCM bits, or a stage without a decoder (zlib, bzip2, sparse, LZMA) are
/// refused.
pub(super) fn decompress_masked(
    mask: u8,
    payload: &[u8],
    expected: usize,
) -> Result<Vec<u8>, CodecError> {
    use compression::*;
    let unsupported = |reason| CodecError {
        codec: "sector",
        reason,
    };
    if mask == LZMA {
        return Err(unsupported("LZMA compression is not supported"));
    }
    if mask & SPARSE != 0 {
        return Err(unsupported("sparse compression is not supported"));
    }
    let known = HUFFMAN | ZLIB | PKWARE | BZIP2 | ADPCM_MONO | ADPCM_STEREO;
    if mask & !known != 0 || mask & (ADPCM_MONO | ADPCM_STEREO) == ADPCM_MONO | ADPCM_STEREO {
        return Err(unsupported("invalid compression mask"));
    }
    if mask & BZIP2 != 0 {
        return Err(unsupported("bzip2 compression is not supported yet"));
    }
    if mask & ZLIB != 0 {
        return Err(unsupported("zlib compression is not supported yet"));
    }

    // Decoders run in the fixed order PKWARE, Huffman, ADPCM (stereo, then mono);
    // each consumes the previous output.
    let mut buf: Option<Vec<u8>> = None;
    if mask & PKWARE != 0 {
        buf = Some(explode::explode(
            buf.as_deref().unwrap_or(payload),
            expected,
        )?);
    }
    if mask & HUFFMAN != 0 {
        buf = Some(huffman::decompress(
            buf.as_deref().unwrap_or(payload),
            expected,
        )?);
    }
    if mask & ADPCM_STEREO != 0 {
        buf = Some(adpcm::decompress(
            buf.as_deref().unwrap_or(payload),
            2,
            expected,
        ));
    }
    if mask & ADPCM_MONO != 0 {
        buf = Some(adpcm::decompress(
            buf.as_deref().unwrap_or(payload),
            1,
            expected,
        ));
    }
    buf.ok_or(unsupported("empty compression mask"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_pkware_sector() {
        let mut sector = vec![compression::PKWARE];
        sector.extend_from_slice(&[0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F]);
        let block = BlockEntry {
            file_pos: 0,
            compressed_size: 0,
            file_size: 13,
            flags: flags::EXISTS | flags::COMPRESS,
        };
        let out = decompress_sector(&block, &sector, 13, None).unwrap();
        assert_eq!(out, b"AIAIAIAIAIAIA");
    }

    #[test]
    fn size_mismatch_is_an_error() {
        let mut sector = vec![compression::PKWARE];
        sector.extend_from_slice(&[0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F]);
        let block = BlockEntry {
            file_pos: 0,
            compressed_size: 0,
            file_size: 20,
            flags: flags::EXISTS | flags::COMPRESS,
        };
        assert!(decompress_sector(&block, &sector, 20, None).is_err());
    }

    #[test]
    fn rejects_unknown_masks() {
        for mask in [0x04, 0x12, 0x20, 0xC0] {
            assert!(
                decompress_masked(mask, &[0; 8], 8).is_err(),
                "mask {mask:#x}"
            );
        }
    }

    #[test]
    fn adpcm_mask_selects_channel_count() {
        // C = 1 for mask 0x40, 2 for mask 0x80; samples are i16 LE.
        let payload = [0x00, 0x00, 0x34, 0x12, 0xCC, 0xED, 0x80, 0x80, 0x07];
        let mono = decompress_masked(compression::ADPCM_MONO, &payload, 64).unwrap();
        assert_eq!(mono, adpcm::decompress(&payload, 1, 64));
        let stereo = decompress_masked(compression::ADPCM_STEREO, &payload, 64).unwrap();
        assert_eq!(stereo, adpcm::decompress(&payload, 2, 64));
        assert_ne!(mono, stereo);
        // Mono: 0x1234, then 0xEDCC read as an op stream; stereo: both
        // initial samples first.
        assert_eq!(mono[..2], 0x1234i16.to_le_bytes());
        assert_eq!(stereo[..4], [0x34, 0x12, 0xCC, 0xED]);
        // Stereo repeats: channel 0 then channel 1.
        assert_eq!(stereo[4..8], [0x34, 0x12, 0xCC, 0xED]);
    }
}

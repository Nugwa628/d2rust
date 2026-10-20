//! Entry points to the crate-private MPQ decompressors for the `fuzz/`
//! cargo-fuzz targets.
//!
//! Each function runs one decoder (or a whole sector) on arbitrary bytes
//! and reports whether it succeeded, so the fuzzers can look for panics,
//! hangs and runaway allocations; the result itself is not checked.
//! Compiled only with the `fuzz` feature, which no game crate enables.

use super::sector::{decompress_masked, decompress_sector};
use super::{adpcm, explode, huffman, BlockEntry};

/// PKWARE DCL explode; `true` if it returned data.
pub fn explode(input: &[u8], max_out: usize) -> bool {
    explode::explode(input, max_out).is_ok()
}

/// MPQ Huffman decompress; `true` if it returned data.
pub fn huffman_decompress(input: &[u8], max_out: usize) -> bool {
    huffman::decompress(input, max_out).is_ok()
}

/// MPQ ADPCM decompress (`channels` 1 or 2); returns the output length.
pub fn adpcm_decompress(input: &[u8], channels: usize, max_out: usize) -> usize {
    adpcm::decompress(input, channels, max_out).len()
}

/// A masked sector payload; `true` if it decoded.
pub fn masked(mask: u8, payload: &[u8], expected: usize) -> bool {
    decompress_masked(mask, payload, expected).is_ok()
}

/// A whole sector under block flags `flags`; `true` if it decoded.
pub fn sector(flags: u32, sector: &[u8], expected: usize) -> bool {
    let block = BlockEntry {
        file_pos: 0,
        compressed_size: sector.len() as u32,
        file_size: expected as u32,
        flags,
    };
    decompress_sector(&block, sector, expected, None).is_ok()
}

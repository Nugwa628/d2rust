//! Read-only access to MPQ archives (format version 0, as used by Diablo II).
//!
//! The game's data lives in `.mpq` archives. [`Archive`] opens one, reads its hash and
//! block tables, and returns a file's bytes by name (decrypting and decompressing sectors
//! as needed). [`ArchiveSet`] stacks the game's archives in search order, and the
//! asset loaders elsewhere in `d2-formats` and D2RUST read through it. Input is a path to
//! an archive on disk; output is the file's bytes or an [`MpqError`].
//!
//! This module holds the on-disk records (header, hash and block entries, flag bits).
//! Submodules hold the reader (`archive`), the name hash and cipher (`crypto`), the
//! sector decompression (`sector`) with its codecs (PKWARE in `explode`, Huffman,
//! ADPCM), and a test-only archive writer.
//!
//! ```no_run
//! # fn main() -> Result<(), d2_formats::mpq::MpqError> {
//! let archive = d2_formats::mpq::Archive::open("game/d2data.mpq")?;
//! let bytes = archive.read(r"data\global\excel\armor.txt")?;
//! # Ok(()) }
//! ```

mod adpcm;
mod archive;
mod bits;
pub mod crypto;
mod explode;
#[cfg(feature = "fuzz")]
pub mod fuzz_api;
mod huffman;
#[cfg(test)]
mod huffman_model;
#[cfg(test)]
mod huffman_tests;
#[cfg(test)]
mod mutant_tests_archive;
#[cfg(test)]
mod mutant_tests_codecs;
pub mod names;
#[cfg(test)]
mod robust_tests;
mod sector;
mod set;
mod tables;
#[cfg(any(test, feature = "test-support"))]
pub mod writer;
#[cfg(test)]
mod writer_tests;

pub use archive::Archive;
pub use set::{
    archive_file_name, priority, search_order, ArchiveSet, ArchiveSpec, OPEN_ORDER, PRIORITY,
};

use std::io;

/// Block table flags.
pub mod flags {
    /// Sectors are bare PKWARE streams (no compression mask byte).
    pub const IMPLODE: u32 = 0x0000_0100;
    /// Sectors start with a compression mask byte.
    pub const COMPRESS: u32 = 0x0000_0200;
    /// The block is encrypted.
    pub const ENCRYPTED: u32 = 0x0001_0000;
    /// The encryption key is adjusted by the block's position and size.
    pub const FIX_KEY: u32 = 0x0002_0000;
    /// The block is a patch file.
    pub const PATCH_FILE: u32 = 0x0010_0000;
    /// The file is stored as one unit instead of sectors.
    pub const SINGLE_UNIT: u32 = 0x0100_0000;
    /// The file is deleted; lookups report it as not found.
    pub const DELETE_MARKER: u32 = 0x0200_0000;
    /// A table of sector checksums follows the sectors.
    pub const SECTOR_CRC: u32 = 0x0400_0000;
    /// The block holds a file.
    pub const EXISTS: u32 = 0x8000_0000;
}

/// Sector compression mask bits (the first byte of a compressed sector).
pub mod compression {
    /// Huffman stage.
    pub const HUFFMAN: u8 = 0x01;
    /// zlib stage (not supported yet).
    pub const ZLIB: u8 = 0x02;
    /// PKWARE DCL stage.
    pub const PKWARE: u8 = 0x08;
    /// bzip2 stage (not supported).
    pub const BZIP2: u8 = 0x10;
    /// Sparse stage (not supported).
    pub const SPARSE: u8 = 0x20;
    /// IMA ADPCM stage, one channel.
    pub const ADPCM_MONO: u8 = 0x40;
    /// IMA ADPCM stage, two channels.
    pub const ADPCM_STEREO: u8 = 0x80;
    /// LZMA (not supported); a whole-byte value, not a bit mask.
    pub const LZMA: u8 = 0x12;
}

const HEADER_MAGIC: [u8; 4] = *b"MPQ\x1A";
const USER_DATA_MAGIC: [u8; 4] = *b"MPQ\x1B";
const HASH_EMPTY: u32 = 0xFFFF_FFFF;
const HASH_DELETED: u32 = 0xFFFF_FFFE;

/// Everything that can go wrong opening an archive or reading a file from it.
#[derive(Debug, thiserror::Error)]
pub enum MpqError {
    /// Reading the archive file failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// No MPQ header was found.
    #[error("not an MPQ archive")]
    NotAnArchive,
    /// The header's format version is not 0.
    #[error("unsupported MPQ format version {0}")]
    UnsupportedVersion(u16),
    /// The header, a table or a sector is inconsistent.
    #[error("corrupt archive: {0}")]
    Corrupt(String),
    /// The name is not in the hash table, or its block is a delete marker.
    #[error("file not found: {0}")]
    NotFound(String),
    /// A block index past the end of the block table.
    #[error("block index {0} out of range")]
    BadBlockIndex(usize),
    /// An encrypted block was read without a key.
    #[error("encrypted block {0} needs a key")]
    KeyRequired(usize),
    /// A feature the reader does not implement.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A sector decompressor rejected its input.
    #[error("block {block}: {source}")]
    Codec {
        /// Index of the block being read.
        block: usize,
        /// The decompressor's error.
        source: CodecError,
    },
}

/// A decompressor rejected its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{codec}: {reason}")]
pub struct CodecError {
    /// Which decompressor failed.
    pub codec: &'static str,
    /// What was wrong with the input.
    pub reason: &'static str,
}

/// The 32-byte archive header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Offset of the archive inside the file; table positions are relative to it.
    pub offset: u64,
    /// Header size field (+0x04); at least 0x20.
    pub header_size: u32,
    /// Archive size field (+0x08).
    pub archive_size: u32,
    /// Format version (+0x0C); only 0 is accepted.
    pub format_version: u16,
    /// Sector size is `0x200 << sector_size_shift`.
    pub sector_size_shift: u16,
    /// Hash table position, relative to `offset`.
    pub hash_table_pos: u32,
    /// Block table position, relative to `offset`.
    pub block_table_pos: u32,
    /// Number of hash entries; a power of two.
    pub hash_table_count: u32,
    /// Number of block entries.
    pub block_table_count: u32,
}

/// One 16-byte hash table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashEntry {
    /// First name check hash.
    pub name_a: u32,
    /// Second name check hash.
    pub name_b: u32,
    /// Locale id (0 is neutral).
    pub locale: u16,
    /// Platform id (0 is the default).
    pub platform: u16,
    /// Index into the block table, or the empty/deleted marker.
    pub block_index: u32,
}

impl HashEntry {
    /// True for a never-used slot, which ends a probe chain.
    pub fn is_empty(&self) -> bool {
        self.block_index == HASH_EMPTY
    }
    /// True for a slot whose file was removed; probing continues past it.
    pub fn is_deleted(&self) -> bool {
        self.block_index == HASH_DELETED
    }
}

/// One 16-byte block table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEntry {
    /// Offset of the block, relative to the archive start.
    pub file_pos: u32,
    /// Stored size in bytes.
    pub compressed_size: u32,
    /// Size after decompression.
    pub file_size: u32,
    /// Bits from [`flags`].
    pub flags: u32,
}

impl BlockEntry {
    /// True if any bit of `flag` is set in [`BlockEntry::flags`].
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
    fn is_compressed(&self) -> bool {
        self.has(flags::COMPRESS | flags::IMPLODE)
    }
}

/// Counts of the compression masks seen while reading, indexed by mask byte.
/// Index 0 counts stored (uncompressed) sectors. Used by survey tooling.
#[derive(Debug, Clone)]
pub struct SectorStats {
    /// Sector count per mask byte.
    pub masks: [u64; 256],
    /// Sectors decompressed with the IMPLODE block flag (no mask byte).
    pub imploded: u64,
}

impl Default for SectorStats {
    fn default() -> Self {
        Self {
            masks: [0; 256],
            imploded: 0,
        }
    }
}

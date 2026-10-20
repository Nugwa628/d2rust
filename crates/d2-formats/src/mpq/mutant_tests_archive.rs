//! Exact-boundary tests for the MPQ archive reader, its tables and the
//! archive set: each pins one rule (an offset, a comparison, a flag test)
//! tightly enough that changing that rule in the code makes a test fail.
//! They cover header search, hash-table lookup, sector reading, key
//! recovery and the archive set. Archives are built here byte by byte and
//! written to temp files. Test-only; the code under test opens the
//! player's game archives.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::crypto::{encrypt, hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
use super::{flags, Archive, ArchiveSet, HashEntry, MpqError, SectorStats};

const HASH_COUNT: usize = 16;
/// PKWARE DCL test vector (a known test vector): decodes to `AIAIAIAIAIAIA`.
const BLAST: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F];
const BLAST_OUT: &[u8] = b"AIAIAIAIAIAIA";

fn unique_path(ext: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "d2-formats-mpq-mutants-{}-{n}{ext}",
        std::process::id()
    ))
}

/// A file in the temp directory, removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> TempFile {
        let path = unique_path(".mpq");
        std::fs::write(&path, bytes).expect("write temp archive");
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A directory in the temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let path = unique_path("");
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(name), bytes).expect("write temp file");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One block: flags, stored bytes, file size. `file_pos` is assigned at
/// assembly.
#[derive(Clone)]
struct Block {
    flags: u32,
    stored: Vec<u8>,
    file_size: u32,
    /// Overrides compressed_size (default: `stored.len()`).
    compressed_size: Option<u32>,
}

/// A plain hash entry: name_a, name_b, locale, platform, block_index.
#[derive(Clone, Copy)]
struct Slot {
    name_a: u32,
    name_b: u32,
    locale: u16,
    platform: u16,
    block_index: u32,
}

const EMPTY: Slot = Slot {
    name_a: 0xFFFF_FFFF,
    name_b: 0xFFFF_FFFF,
    locale: 0xFFFF,
    platform: 0xFFFF,
    block_index: 0xFFFF_FFFF,
};

/// A synthetic archive built by hand (header, hash table, block table).
#[derive(Clone)]
struct Builder {
    /// Bytes before the header (`A = prefix.len()`).
    prefix: Vec<u8>,
    header_size: u32,
    archive_size: u32,
    version: u16,
    shift: u16,
    slots: Vec<Slot>,
    blocks: Vec<Block>,
    /// Put the tables before the file data (so the last block ends the file).
    tables_first: bool,
}

impl Builder {
    fn new() -> Builder {
        Builder {
            prefix: Vec::new(),
            header_size: 0x20,
            archive_size: 0,
            version: 0,
            shift: 0,
            slots: vec![EMPTY; HASH_COUNT],
            blocks: Vec::new(),
            tables_first: false,
        }
    }

    fn slot_of(name: &str, locale: u16, block_index: u32) -> Slot {
        Slot {
            name_a: hash(name.as_bytes(), HashType::NameA),
            name_b: hash(name.as_bytes(), HashType::NameB),
            locale,
            platform: 0,
            block_index,
        }
    }

    fn start(name: &str) -> usize {
        hash(name.as_bytes(), HashType::TableOffset) as usize & (HASH_COUNT - 1)
    }

    /// Adds a block and returns its index.
    fn block(&mut self, flags: u32, stored: Vec<u8>, file_size: u32) -> u32 {
        self.blocks.push(Block {
            flags,
            stored,
            file_size,
            compressed_size: None,
        });
        (self.blocks.len() - 1) as u32
    }

    /// Adds a block and a hash entry for `name` at the first free slot of
    /// its probe sequence.
    fn file(&mut self, name: &str, flags: u32, stored: Vec<u8>, file_size: u32) -> u32 {
        let index = self.block(flags, stored, file_size);
        let start = Self::start(name);
        let slot = (0..HASH_COUNT)
            .map(|i| (start + i) % HASH_COUNT)
            .find(|&s| self.slots[s].block_index == 0xFFFF_FFFF)
            .expect("free slot");
        self.slots[slot] = Self::slot_of(name, 0, index);
        index
    }

    /// A stored (uncompressed, unencrypted) file.
    fn stored(&mut self, name: &str, contents: &[u8]) -> u32 {
        self.file(
            name,
            flags::EXISTS,
            contents.to_vec(),
            contents.len() as u32,
        )
    }

    fn bytes(&self) -> Vec<u8> {
        let a = self.prefix.len();
        let mut hash_t: Vec<u8> = self
            .slots
            .iter()
            .flat_map(|s| {
                let mut e = Vec::with_capacity(16);
                e.extend_from_slice(&s.name_a.to_le_bytes());
                e.extend_from_slice(&s.name_b.to_le_bytes());
                e.extend_from_slice(&s.locale.to_le_bytes());
                e.extend_from_slice(&s.platform.to_le_bytes());
                e.extend_from_slice(&s.block_index.to_le_bytes());
                e
            })
            .collect();
        let hash_len = hash_t.len();
        let block_len = self.blocks.len() * 16;
        let data_len: usize = self.blocks.iter().map(|b| b.stored.len()).sum();
        let (data_pos, hash_pos) = if self.tables_first {
            (32 + hash_len + block_len, 32)
        } else {
            (32, 32 + data_len)
        };
        let block_pos = hash_pos + hash_len;

        let mut data = Vec::new();
        let mut block_t = Vec::new();
        for b in &self.blocks {
            let file_pos = (data_pos + data.len()) as u32;
            let cs = b.compressed_size.unwrap_or(b.stored.len() as u32);
            for v in [file_pos, cs, b.file_size, b.flags] {
                block_t.extend_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(&b.stored);
        }
        encrypt(&mut hash_t, HASH_TABLE_KEY);
        encrypt(&mut block_t, BLOCK_TABLE_KEY);

        let mut out = self.prefix.clone();
        out.extend_from_slice(b"MPQ\x1A");
        out.extend_from_slice(&self.header_size.to_le_bytes());
        out.extend_from_slice(&self.archive_size.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.shift.to_le_bytes());
        for v in [
            hash_pos as u32,
            block_pos as u32,
            HASH_COUNT as u32,
            self.blocks.len() as u32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        if self.tables_first {
            out.extend_from_slice(&hash_t);
            out.extend_from_slice(&block_t);
            out.extend_from_slice(&data);
        } else {
            out.extend_from_slice(&data);
            out.extend_from_slice(&hash_t);
            out.extend_from_slice(&block_t);
        }
        assert_eq!(out.len(), a + 32 + hash_len + block_len + data_len);
        out
    }
}

fn open(bytes: &[u8]) -> (TempFile, Result<Archive, MpqError>) {
    let tmp = TempFile::new(bytes);
    let a = Archive::open(&tmp.0);
    (tmp, a)
}

/// A one-file archive behind `prefix`.
fn small_archive(prefix: Vec<u8>) -> Vec<u8> {
    let mut b = Builder::new();
    b.prefix = prefix;
    b.stored("x.txt", b"hello");
    b.bytes()
}

// ---------------------------------------------------------------------------
// Archive header.

/// The archive starts at the first 0x200-aligned offset holding
/// "MPQ\x1A"; magic at an unaligned offset is not a header.
#[test]
fn header_at_first_aligned_magic() {
    let mut prefix = vec![0u8; 0x200];
    prefix[0x100..0x104].copy_from_slice(b"MPQ\x1A");
    let (_t, a) = open(&small_archive(prefix));
    let a = a.unwrap();
    assert_eq!(a.header().offset, 0x200);
    assert_eq!(a.read("x.txt").unwrap(), b"hello");
}

/// The header search continues past the first megabyte: the reader scans the
/// whole file.
#[test]
fn header_beyond_first_megabyte() {
    let at = 0x10_0000 + 0x400;
    let (_t, a) = open(&small_archive(vec![0u8; at]));
    let a = a.unwrap();
    assert_eq!(a.header().offset, at as u64);
    assert_eq!(a.read("x.txt").unwrap(), b"hello");
}

/// A file with no "MPQ\x1A" at any aligned offset is not an archive.
#[test]
fn no_header_is_not_an_archive() {
    let mut bytes = vec![0u8; 0x900];
    bytes[0x10..0x14].copy_from_slice(b"MPQ\x1A");
    let (_t, a) = open(&bytes);
    assert!(matches!(a, Err(MpqError::NotAnArchive)));
}

/// A user-data header ("MPQ\x1B") at offset 0 redirects to
/// `candidate + header_offset`.
#[test]
fn user_data_header_at_zero() {
    let mut prefix = vec![0u8; 0x200];
    prefix[0..4].copy_from_slice(b"MPQ\x1B");
    prefix[8..12].copy_from_slice(&0x200u32.to_le_bytes());
    let (_t, a) = open(&small_archive(prefix));
    let a = a.unwrap();
    assert_eq!(a.header().offset, 0x200);
    assert_eq!(a.read("x.txt").unwrap(), b"hello");
}

/// A user-data header at 0x200 with header offset 0x400 puts the
/// archive at 0x600. The scan stops there: an archive header at 0x400 (a
/// decoy, version 1) is never reached.
#[test]
fn user_data_header_redirects_past_later_magic() {
    let mut prefix = vec![0u8; 0x600];
    prefix[0x200..0x204].copy_from_slice(b"MPQ\x1B");
    prefix[0x208..0x20C].copy_from_slice(&0x400u32.to_le_bytes());
    prefix[0x400..0x404].copy_from_slice(b"MPQ\x1A");
    prefix[0x404..0x408].copy_from_slice(&0x20u32.to_le_bytes());
    prefix[0x40C..0x40E].copy_from_slice(&1u16.to_le_bytes());
    let bytes = small_archive(prefix);
    assert!(bytes.len() < 0x800);
    let (_t, a) = open(&bytes);
    let a = a.unwrap();
    assert_eq!(a.header().offset, 0x600);
    assert_eq!(a.read("x.txt").unwrap(), b"hello");
}

/// Header fields: header_size ≥ 0x20 (larger is fine, smaller is
/// corrupt); archive_size is informational, not validated; format version
/// other than 0 is rejected; sector size = 0x200 << shift.
#[test]
fn header_field_rules() {
    let mut b = Builder::new();
    b.stored("x.txt", b"hello");
    b.header_size = 0x2C;
    b.archive_size = 0xFF12_3456;
    b.shift = 3;
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    let h = a.header();
    assert_eq!(
        (h.header_size, h.archive_size, h.format_version),
        (0x2C, 0xFF12_3456, 0)
    );
    assert_eq!(h.sector_size_shift, 3);
    assert_eq!(a.sector_size(), 0x1000);
    assert_eq!(a.read("x.txt").unwrap(), b"hello");

    let mut small = b.clone();
    small.header_size = 0x1C;
    let (_t, a) = open(&small.bytes());
    assert!(matches!(a, Err(MpqError::Corrupt(_))));

    for version in [1u16, 0x100] {
        let mut v = b.clone();
        v.version = version;
        let (_t, a) = open(&v.bytes());
        assert!(
            matches!(a, Err(MpqError::UnsupportedVersion(x)) if x == version),
            "version {version:#x}"
        );
    }
}

/// A 32-byte file whose tables lie inside it (here, overlapping the
/// header) is a valid archive: the header fits exactly.
#[test]
fn header_ending_at_end_of_file() {
    let mut h = b"MPQ\x1A".to_vec();
    for v in [0x20u32, 0x20] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.extend_from_slice(&[0, 0, 0, 0]);
    for v in [0u32, 0, 1, 0] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(h.len(), 32);
    let (_t, a) = open(&h);
    let a = a.unwrap();
    assert_eq!(a.header().offset, 0);
    assert_eq!(a.hash_table().len(), 1);
    assert!(a.block_table().is_empty());
}

// ---------------------------------------------------------------------------
// Hash table.

/// Entries decrypted with the hash-table key, fields at their offsets.
#[test]
fn hash_table_entries_decrypted() {
    let mut b = Builder::new();
    b.slots[3] = Slot {
        name_a: 0x0102_0304,
        name_b: 0x0506_0708,
        locale: 0x0409,
        platform: 0x0A0B,
        block_index: 0xFFFF_FFFE,
    };
    b.stored("x.txt", b"hello");
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    let t = a.hash_table();
    assert_eq!(t.len(), HASH_COUNT);
    assert_eq!(
        t[3],
        HashEntry {
            name_a: 0x0102_0304,
            name_b: 0x0506_0708,
            locale: 0x0409,
            platform: 0x0A0B,
            block_index: 0xFFFF_FFFE,
        }
    );
    assert!(t[3].is_deleted() && !t[3].is_empty());
    let start = Builder::start("x.txt");
    let s = if start == 3 { 4 } else { start };
    assert_eq!(t[s].name_a, hash(b"x.txt", HashType::NameA));
    assert_eq!(t[s].name_b, hash(b"x.txt", HashType::NameB));
    assert_eq!((t[s].locale, t[s].platform, t[s].block_index), (0, 0, 0));
}

/// Lookup: the probe stops at an empty entry and skips deleted
/// ones.
#[test]
fn lookup_stops_at_empty_and_skips_deleted() {
    let name = "y.dat";
    let start = Builder::start(name);
    let mut b = Builder::new();
    let index = b.block(flags::EXISTS, b"why".to_vec(), 3);
    b.slots[(start + 1) % HASH_COUNT] = Builder::slot_of(name, 0, index);

    // Slot `start` empty: the match after it is never reached.
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert!(a.hash_table()[start].is_empty());
    assert_eq!(a.find(name), None);
    assert!(!a.contains(name));
    assert!(matches!(a.read(name), Err(MpqError::NotFound(_))));

    // Slot `start` deleted: skipped, the match after it is found.
    b.slots[start] = Slot {
        block_index: 0xFFFF_FFFE,
        ..EMPTY
    };
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.find(name), Some(index as usize));
    assert!(a.contains(name));
    assert_eq!(a.read(name).unwrap(), b"why");
}

/// Lookup: among matches, the first with locale 0 and
/// platform 0; any other locale or platform is never returned, even as
/// the only match. A block index out of range doesn't match.
#[test]
fn lookup_takes_neutral_locale_only() {
    let name = "z.bin";
    let start = Builder::start(name);
    let at = |i: usize| (start + i) % HASH_COUNT;
    let mut b = Builder::new();
    let german = b.block(flags::EXISTS, b"de".to_vec(), 2);
    let neutral = b.block(flags::EXISTS, b"neutral".to_vec(), 7);
    let french = b.block(flags::EXISTS, b"fr".to_vec(), 2);
    b.slots[at(0)] = Builder::slot_of(name, 0, 99); // out of range
    b.slots[at(1)] = Builder::slot_of(name, 0x0407, german);
    b.slots[at(2)] = Builder::slot_of(name, 0, neutral);
    b.slots[at(3)] = Builder::slot_of(name, 0x040C, french);
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.find(name), Some(neutral as usize));
    assert_eq!(a.read(name).unwrap(), b"neutral");

    // No locale-0 match: not found.
    b.slots[at(2)] = Builder::slot_of(name, 0x0410, neutral);
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.find(name), None);

    // Locale 0 on a nonzero platform: skipped, the next neutral one wins.
    b.slots[at(2)] = Slot {
        platform: 1,
        ..Builder::slot_of(name, 0, neutral)
    };
    b.slots[at(3)] = Builder::slot_of(name, 0, french);
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.find(name), Some(french as usize));
}

/// A name not in the table, or whose block is DELETE_MARKER, is
/// not contained.
#[test]
fn contains_only_present_files() {
    let mut b = Builder::new();
    b.stored("x.txt", b"hello");
    b.file(
        "gone.txt",
        flags::EXISTS | flags::DELETE_MARKER,
        b"g".to_vec(),
        1,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert!(a.contains("x.txt"));
    assert!(a.contains("X.TXT"));
    assert!(!a.contains("missing.txt"));
    assert!(!a.contains("gone.txt"));
}

// ---------------------------------------------------------------------------
// Reading file data.

/// The data range must lie inside the file; a block ending exactly at
/// the end of the file is inside.
#[test]
fn block_ending_at_end_of_file() {
    let mut b = Builder::new();
    b.tables_first = true;
    b.stored("first.txt", b"one");
    b.stored("last.txt", b"the last bytes");
    let bytes = b.bytes();
    assert!(bytes.ends_with(b"the last bytes"));
    let (_t, a) = open(&bytes);
    let a = a.unwrap();
    assert_eq!(a.read("last.txt").unwrap(), b"the last bytes");
    assert_eq!(a.read("first.txt").unwrap(), b"one");
}

/// SINGLE_UNIT: compressed only when COMPRESS/IMPLODE is set and
/// `compressed_size < file_size`; otherwise stored. A stored unit shorter
/// than the file is corrupt (the result must be `file_size` bytes).
#[test]
fn single_unit_stored_and_compressed() {
    // 0x04 is not an allowed mask bit: decoding these bytes would fail.
    let raw = b"\x04 stored as-is".to_vec();
    let n = raw.len() as u32;
    let mut b = Builder::new();
    b.file("plain", flags::EXISTS | flags::SINGLE_UNIT, raw.clone(), n);
    b.file(
        "same.size",
        flags::EXISTS | flags::SINGLE_UNIT | flags::COMPRESS,
        raw.clone(),
        n,
    );
    let mut packed = vec![0x08];
    packed.extend_from_slice(&BLAST);
    b.file(
        "packed",
        flags::EXISTS | flags::SINGLE_UNIT | flags::COMPRESS,
        packed,
        BLAST_OUT.len() as u32,
    );
    b.file(
        "short",
        flags::EXISTS | flags::SINGLE_UNIT,
        raw.clone(),
        n + 4,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.read("plain").unwrap(), raw);
    assert_eq!(a.read("same.size").unwrap(), raw);
    assert_eq!(a.read("packed").unwrap(), BLAST_OUT);
    assert!(a.read("short").is_err());
}

/// Uncompressed: `compressed_size ≥ file_size`; the result is exactly
/// `file_size` bytes.
#[test]
fn uncompressed_longer_range_is_cut_to_file_size() {
    let mut b = Builder::new();
    b.file("long", flags::EXISTS, b"0123456789abcdef".to_vec(), 10);
    b.file("short", flags::EXISTS, b"0123".to_vec(), 10);
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.read("long").unwrap(), b"0123456789");
    assert!(a.read("short").is_err());
}

/// Sector offsets never decrease (equal is allowed), and the
/// SECTOR_CRC block is ignored, even when it is empty.
#[test]
fn empty_sector_crc_block() {
    // One sector of 13 bytes, PKWARE-compressed (mask 0x08), table of N + 2
    // entries with offset[N + 1] == offset[N].
    let mut sector = vec![0x08];
    sector.extend_from_slice(&BLAST);
    let table_len = 3 * 4;
    let end = (table_len + sector.len()) as u32;
    let mut stored = Vec::new();
    for o in [table_len as u32, end, end] {
        stored.extend_from_slice(&o.to_le_bytes());
    }
    stored.extend_from_slice(&sector);
    let mut b = Builder::new();
    b.file(
        "crc.bin",
        flags::EXISTS | flags::COMPRESS | flags::SECTOR_CRC,
        stored,
        BLAST_OUT.len() as u32,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.read("crc.bin").unwrap(), BLAST_OUT);
}

/// `read_block_stats` reads the same bytes as `read_block` on every
/// sector kind: stored, raw under COMPRESS, masked, imploded.
#[test]
fn read_block_stats_returns_file_data() {
    let sectored = |sectors: &[Vec<u8>]| {
        let table_len = (sectors.len() + 1) * 4;
        let mut offsets = vec![table_len as u32];
        for s in sectors {
            offsets.push(offsets.last().unwrap() + s.len() as u32);
        }
        let mut out: Vec<u8> = offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
        for s in sectors {
            out.extend_from_slice(s);
        }
        out
    };
    let mut masked = vec![0x08];
    masked.extend_from_slice(&BLAST);
    let mut b = Builder::new();
    let stored = b.stored("stored", b"plain data");
    let raw = b.file(
        "raw",
        flags::EXISTS | flags::COMPRESS,
        sectored(&[b"\x04raw".to_vec()]),
        4,
    );
    let masked = b.file(
        "masked",
        flags::EXISTS | flags::COMPRESS,
        sectored(&[masked]),
        13,
    );
    let imploded = b.file(
        "imploded",
        flags::EXISTS | flags::IMPLODE,
        sectored(&[BLAST.to_vec()]),
        13,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    let mut stats = SectorStats::default();
    for (i, want) in [
        (stored, &b"plain data"[..]),
        (raw, b"\x04raw"),
        (masked, BLAST_OUT),
        (imploded, BLAST_OUT),
    ] {
        let i = i as usize;
        assert_eq!(a.read_block(i, None).unwrap(), want);
        assert_eq!(a.read_block_stats(i, None, &mut stats).unwrap(), want);
    }
}

// ---------------------------------------------------------------------------
// Key recovery.

/// Key recovery applies to encrypted, compressed, non-single-unit files (only
/// encrypted files have a key). An encrypted uncompressed block, or an
/// encrypted single-unit block (stored under COMPRESS), whose data
/// starts like an offset table (`P`, then a small dword) has no key to
/// recover this way.
#[test]
fn recover_key_only_for_compressed_blocks() {
    let file_size = 0x300u32; // 2 sectors of 0x200: P = 3 * 4.
    let mut plain = vec![0u8; file_size as usize];
    plain[0..4].copy_from_slice(&12u32.to_le_bytes());
    plain[4..8].copy_from_slice(&0u32.to_le_bytes());
    let key = hash(b"enc.dat", HashType::FileKey);
    let mut stored = plain.clone();
    for (i, chunk) in stored.chunks_mut(0x200).enumerate() {
        encrypt(chunk, key.wrapping_add(i as u32));
    }
    let mut b = Builder::new();
    let index = b.file(
        "enc.dat",
        flags::EXISTS | flags::ENCRYPTED,
        stored,
        file_size,
    );
    let mut unit = plain.clone();
    let unit_key = hash(b"unit.dat", HashType::FileKey);
    encrypt(&mut unit, unit_key);
    let single = b.file(
        "unit.dat",
        flags::EXISTS | flags::ENCRYPTED | flags::COMPRESS | flags::SINGLE_UNIT,
        unit,
        file_size,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.read("enc.dat").unwrap(), plain);
    assert_eq!(a.recover_key(index as usize).unwrap(), None);
    assert_eq!(a.read("unit.dat").unwrap(), plain);
    assert_eq!(a.recover_key(single as usize).unwrap(), None);
}

// ---------------------------------------------------------------------------
// Archive set.

/// Archive set: only the known archives are opened, in search order;
/// lookups take the first archive that has the file.
#[test]
fn archive_set_open_dir_and_lookup() {
    let dir = TempDir::new();
    let mut data = Builder::new();
    data.stored("shared.txt", b"from d2data");
    data.stored("data-only.txt", b"only in d2data");
    let mut patch = Builder::new();
    patch.stored("shared.txt", b"from patch_d2");
    dir.put("d2data.mpq", &data.bytes());
    dir.put("patch_d2.mpq", &patch.bytes());
    // Not a known archive name, and not an archive: never opened.
    dir.put("notes.mpq", b"not an archive");
    dir.put("readme.txt", b"hello");

    let set = ArchiveSet::open_dir(&dir.0).unwrap();
    let names: Vec<&str> = set
        .archives()
        .iter()
        .map(super::archive_file_name)
        .collect();
    assert_eq!(names, ["patch_d2.mpq", "d2data.mpq"]);
    assert!(set.has_archive("d2data.mpq"));
    assert!(set.has_archive("patch_d2.mpq"));
    assert!(!set.has_archive("d2exp.mpq"));

    assert!(set.contains("shared.txt"));
    assert!(set.contains("data-only.txt"));
    assert!(!set.contains("missing.txt"));
    let found = set.find("data-only.txt").unwrap();
    assert_eq!(super::archive_file_name(found), "d2data.mpq");
    assert!(set.find("missing.txt").is_none());

    assert_eq!(set.read("shared.txt").unwrap(), b"from patch_d2");
    assert_eq!(set.read("data-only.txt").unwrap(), b"only in d2data");
    assert!(matches!(
        set.read("missing.txt"),
        Err(MpqError::NotFound(_))
    ));

    assert_eq!(
        set.read_with_source("shared.txt").unwrap(),
        Some(("patch_d2.mpq".to_owned(), b"from patch_d2".to_vec()))
    );
    assert_eq!(
        set.read_with_source("data-only.txt").unwrap(),
        Some(("d2data.mpq".to_owned(), b"only in d2data".to_vec()))
    );
    assert_eq!(set.read_with_source("missing.txt").unwrap(), None);
}

/// A directory without known archives opens an empty set.
#[test]
fn archive_set_without_known_archives() {
    let dir = TempDir::new();
    dir.put("other.mpq", &small_archive(Vec::new()));
    let set = ArchiveSet::open_dir(&dir.0).unwrap();
    assert!(set.archives().is_empty());
    assert!(!set.contains("x.txt"));
}

/// Lookup: an entry matches only if `block_index <
/// block_table_count`. An entry whose block index equals the count is
/// passed over: the probe goes on to the next match, or ends not found.
#[test]
fn lookup_rejects_block_index_equal_to_count() {
    let name = "w.txt";
    let start = Builder::start(name);
    let at = |i: usize| (start + i) % HASH_COUNT;
    let mut b = Builder::new();
    let real = b.block(flags::EXISTS, b"real".to_vec(), 4);
    let count = 1u32; // one block
    b.slots[at(0)] = Builder::slot_of(name, 0, count);
    b.slots[at(1)] = Builder::slot_of(name, 0, real);
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.block_table().len(), count as usize);
    assert_eq!(a.find(name), Some(real as usize));
    assert_eq!(a.read(name).unwrap(), b"real");

    // Only the out-of-range entry: not found.
    b.slots[at(1)] = EMPTY;
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert_eq!(a.find(name), None);
    assert!(matches!(a.read(name), Err(MpqError::NotFound(_))));
}

/// Offsets never decrease, and `offset[N] ≤ compressed_size`.
/// Either violation alone makes the file corrupt.
#[test]
fn sector_offsets_validated() {
    // Two raw sectors of a 0x300-byte file (sector size 0x200) would need
    // 0x300 bytes; these tables are rejected before any sector is used.
    let block = |offsets: [u32; 3], data_len: usize| {
        let mut s: Vec<u8> = offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
        s.resize(data_len, 0x41);
        s
    };
    let mut b = Builder::new();
    // Decreasing (12 → 30 → 20), last offset inside the data.
    let dec = b.file(
        "dec",
        flags::EXISTS | flags::COMPRESS,
        block([12, 30, 20], 40),
        0x300,
    );
    // Increasing, last offset past the data.
    let past = b.file(
        "past",
        flags::EXISTS | flags::COMPRESS,
        block([12, 20, 50], 40),
        0x300,
    );
    let (_t, a) = open(&b.bytes());
    let a = a.unwrap();
    assert!(matches!(
        a.read_block(dec as usize, None),
        Err(MpqError::Corrupt(_))
    ));
    assert!(matches!(
        a.read_block(past as usize, None),
        Err(MpqError::Corrupt(_))
    ));
}

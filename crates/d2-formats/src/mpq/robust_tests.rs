//! Robustness property tests for the MPQ reader and its decompressors:
//! malformed input returns an error; it never panics, hangs
//! or allocates beyond what the input allows. Valid inputs are built here
//! by hand (header, tables, sector reading and each codec) and then mutated.
//! Test-only; the code under test reads the player's game archives.
//!
//! `PROPTEST_CASES` raises the case counts for a deeper local run.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use proptest::prelude::*;

use super::bits::BitWriter;
use super::crypto::{encrypt, hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
use super::huffman_model::Model;
use super::sector::{decompress_masked, decompress_sector};
use super::tables::{DIST_BITS, DIST_CODE, EX_LEN_BITS, LEN_BASE, LEN_BITS, LEN_CODE};
use super::{adpcm, compression, explode, flags, huffman, Archive, BlockEntry, MpqError};
use crate::robust::{bounded, bytes, mutated};

/// Case count: `PROPTEST_CASES` if set, else `default`.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn config(default: u32) -> ProptestConfig {
    ProptestConfig {
        cases: cases(default),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Output caps: small, around a sector, and absurd.
fn max_out() -> impl Strategy<Value = usize> {
    prop_oneof![
        0usize..64,
        0usize..0x2000,
        Just(u32::MAX as usize),
        Just(usize::MAX),
        any::<usize>(),
    ]
}

// ---------------------------------------------------------------------------
// Valid stream builders.

/// One PKWARE DCL token.
#[derive(Clone, Copy)]
enum Tok {
    Lit(u8),
    /// Copy `n` bytes from `back` bytes back.
    Copy {
        n: usize,
        back: usize,
    },
}

fn write_len(w: &mut BitWriter, l: u32) {
    let sym = (0..16)
        .rev()
        .find(|&s| u32::from(LEN_BASE[s]) <= l)
        .expect("length symbol");
    w.write(u32::from(LEN_CODE[sym]), u32::from(LEN_BITS[sym]));
    if EX_LEN_BITS[sym] > 0 {
        w.write(l - u32::from(LEN_BASE[sym]), u32::from(EX_LEN_BITS[sym]));
    }
}

/// A binary-mode PKWARE DCL stream of `toks`, ending with the end
/// marker.
fn implode(dict_bits: u32, toks: &[Tok]) -> Vec<u8> {
    let mut w = BitWriter::default();
    for &t in toks {
        match t {
            Tok::Lit(b) => {
                w.write(0, 1);
                w.write(u32::from(b), 8);
            }
            Tok::Copy { n, back } => {
                w.write(1, 1);
                write_len(&mut w, (n - 2) as u32);
                let dist = (back - 1) as u32;
                let low = if n == 2 { 2 } else { dict_bits };
                let dsym = (dist >> low) as usize;
                w.write(u32::from(DIST_CODE[dsym]), u32::from(DIST_BITS[dsym]));
                w.write(dist & ((1 << low) - 1), low);
            }
        }
    }
    w.write(1, 1);
    write_len(&mut w, 0x205);
    let mut out = vec![0, dict_bits as u8];
    out.extend_from_slice(&w.bytes);
    out
}

fn literals(data: &[u8]) -> Vec<Tok> {
    data.iter().map(|&b| Tok::Lit(b)).collect()
}

fn text(len: usize) -> Vec<u8> {
    b"Stay awhile and listen. "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

/// An ADPCM stream for `channels`, with every command kind, that
/// decodes to at least `samples` samples.
fn adpcm_stream(channels: usize, samples: usize) -> Vec<u8> {
    let mut s = vec![0x00, 0x03];
    for c in 0..channels {
        s.extend_from_slice(&(100 * c as i16 - 50).to_le_bytes());
    }
    let ops = [
        0x05, 0x45, 0x81, 0x1F, 0x80, 0x5F, 0x83, 0x02, 0x82, 0x3C, 0x7F,
    ];
    let mut produced = channels;
    let mut i = 0;
    while produced < samples {
        let op = ops[i % ops.len()];
        if op & 0x80 == 0 || op == 0x80 {
            produced += 1;
        }
        s.push(op);
        i += 1;
    }
    s
}

// ---------------------------------------------------------------------------
// Synthetic archive, sector size 0x200.

const SECTOR: usize = 0x200;
const HASH_COUNT: usize = 16;

/// The names stored in the synthetic archive.
const NAMES: [&str; 7] = [
    "(listfile)",
    "a.txt",
    "b.bin",
    "c.wav",
    r"sub\d.dat",
    "e.imp",
    "f.nil",
];

/// The plaintext parts of an archive; [`Parts::assemble`] encrypts the
/// tables, so mutations of the plain tables reach the reader decrypted.
#[derive(Clone)]
struct Parts {
    header: Vec<u8>,
    data: Vec<u8>,
    hash_plain: Vec<u8>,
    block_plain: Vec<u8>,
}

impl Parts {
    fn assemble(&self) -> Vec<u8> {
        let mut hash_t = self.hash_plain.clone();
        encrypt(&mut hash_t, HASH_TABLE_KEY);
        let mut block_t = self.block_plain.clone();
        encrypt(&mut block_t, BLOCK_TABLE_KEY);
        let mut out = self.header.clone();
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&hash_t);
        out.extend_from_slice(&block_t);
        out
    }
}

fn key_for(name: &str, block_flags: u32, file_pos: u32, file_size: u32) -> u32 {
    let plain = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let base = hash(plain.as_bytes(), HashType::FileKey);
    if block_flags & flags::FIX_KEY != 0 {
        base.wrapping_add(file_pos) ^ file_size
    } else {
        base
    }
}

/// A sectored block: offset table, sectors, optional CRC block.
fn sectored(sectors: &[Vec<u8>], crc: bool, key: Option<u32>) -> Vec<u8> {
    let entries = sectors.len() + 1 + usize::from(crc);
    let mut offsets = Vec::new();
    let mut pos = entries * 4;
    for s in sectors {
        offsets.push(pos as u32);
        pos += s.len();
    }
    offsets.push(pos as u32);
    if crc {
        offsets.push(pos as u32 + 4);
    }
    let mut table: Vec<u8> = offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
    if let Some(k) = key {
        encrypt(&mut table, k.wrapping_sub(1));
    }
    let mut out = table;
    for (i, s) in sectors.iter().enumerate() {
        let mut s = s.clone();
        if let Some(k) = key {
            encrypt(&mut s, k.wrapping_add(i as u32));
        }
        out.extend_from_slice(&s);
    }
    if crc {
        out.extend_from_slice(&[0xC0, 0xFF, 0xEE, 0x00]);
    }
    out
}

fn masked(mask: u8, payload: &[u8]) -> Vec<u8> {
    let mut s = vec![mask];
    s.extend_from_slice(payload);
    s
}

/// File contents of the synthetic archive, by name.
fn contents(name: &str) -> Vec<u8> {
    match name {
        "(listfile)" => NAMES[1..].join("\r\n").into_bytes(),
        "a.txt" => text(700),
        "b.bin" => {
            let mut c: Vec<u8> = b"AB".iter().copied().cycle().take(SECTOR).collect();
            c.extend(text(SECTOR + 5)[5..].iter().copied());
            c.extend((0..276u32).map(|i| (i * 37 + 11) as u8));
            c
        }
        "c.wav" => {
            let mut c = adpcm::decompress(&adpcm_stream(1, SECTOR / 2), 1, SECTOR);
            c.extend(adpcm::decompress(&adpcm_stream(2, SECTOR / 2), 2, SECTOR));
            c
        }
        r"sub\d.dat" => (0..700u32).map(|i| (i * 13) as u8).collect(),
        "e.imp" => {
            let mut c = vec![b'x'; SECTOR];
            c.extend(vec![b'y'; 88]);
            c
        }
        "f.nil" => Vec::new(),
        _ => unreachable!("{name}"),
    }
}

/// Block flags and stored bytes of `name` at `file_pos`.
fn stored(name: &str, file_pos: u32) -> (u32, Vec<u8>) {
    use flags::*;
    let c = contents(name);
    let size = c.len() as u32;
    match name {
        "a.txt" => {
            let f = EXISTS | SINGLE_UNIT | COMPRESS | ENCRYPTED;
            let mut s = masked(compression::HUFFMAN, &huffman::compress(0, &c));
            assert!(s.len() < c.len());
            encrypt(&mut s, key_for(name, f, file_pos, size));
            (f, s)
        }
        "b.bin" => {
            let f = EXISTS | COMPRESS | ENCRYPTED | FIX_KEY | SECTOR_CRC;
            let s0 = masked(
                compression::PKWARE,
                &implode(
                    4,
                    &[
                        Tok::Lit(b'A'),
                        Tok::Lit(b'B'),
                        Tok::Copy { n: 510, back: 2 },
                    ],
                ),
            );
            let h = huffman::compress(0, &c[SECTOR..2 * SECTOR]);
            let s1 = masked(
                compression::PKWARE | compression::HUFFMAN,
                &implode(5, &literals(&h)),
            );
            assert!(s1.len() < SECTOR);
            let s2 = c[2 * SECTOR..].to_vec();
            let key = key_for(name, f, file_pos, size);
            (f, sectored(&[s0, s1, s2], true, Some(key)))
        }
        "c.wav" => {
            let f = EXISTS | COMPRESS;
            let s0 = masked(
                compression::ADPCM_MONO | compression::HUFFMAN,
                &huffman::compress(0, &adpcm_stream(1, SECTOR / 2)),
            );
            let s1 = masked(compression::ADPCM_STEREO, &adpcm_stream(2, SECTOR / 2));
            assert!(s0.len() < SECTOR && s1.len() < SECTOR);
            (f, sectored(&[s0, s1], false, None))
        }
        r"sub\d.dat" => {
            let f = EXISTS | ENCRYPTED;
            let key = key_for(name, f, file_pos, size);
            let mut s = c;
            for (i, chunk) in s.chunks_mut(SECTOR).enumerate() {
                encrypt(chunk, key.wrapping_add(i as u32));
            }
            (f, s)
        }
        "e.imp" => {
            let f = EXISTS | IMPLODE;
            let s0 = implode(6, &[Tok::Lit(b'x'), Tok::Copy { n: 511, back: 1 }]);
            let s1 = implode(4, &[Tok::Lit(b'y'), Tok::Copy { n: 87, back: 1 }]);
            (f, sectored(&[s0, s1], false, None))
        }
        _ => (EXISTS, c),
    }
}

fn synthetic_parts() -> Parts {
    let mut data = Vec::new();
    let mut blocks = Vec::new();
    for name in NAMES {
        let file_pos = (32 + data.len()) as u32;
        let (f, s) = stored(name, file_pos);
        blocks.push(BlockEntry {
            file_pos,
            compressed_size: s.len() as u32,
            file_size: contents(name).len() as u32,
            flags: f,
        });
        data.extend_from_slice(&s);
    }

    let mut hash_plain = vec![0xFFu8; HASH_COUNT * 16];
    for (index, name) in NAMES.iter().enumerate() {
        let start = hash(name.as_bytes(), HashType::TableOffset) as usize;
        let slot = (0..HASH_COUNT)
            .map(|i| (start + i) % HASH_COUNT)
            .find(|&s| hash_plain[s * 16 + 12..s * 16 + 16] == [0xFF; 4])
            .expect("free hash slot");
        let e = &mut hash_plain[slot * 16..slot * 16 + 16];
        e[0..4].copy_from_slice(&hash(name.as_bytes(), HashType::NameA).to_le_bytes());
        e[4..8].copy_from_slice(&hash(name.as_bytes(), HashType::NameB).to_le_bytes());
        e[8..12].fill(0);
        e[12..16].copy_from_slice(&(index as u32).to_le_bytes());
    }
    let block_plain: Vec<u8> = blocks
        .iter()
        .flat_map(|b| [b.file_pos, b.compressed_size, b.file_size, b.flags])
        .flat_map(u32::to_le_bytes)
        .collect();

    let hash_pos = (32 + data.len()) as u32;
    let block_pos = hash_pos + hash_plain.len() as u32;
    let total = block_pos + block_plain.len() as u32;
    let mut header = b"MPQ\x1A".to_vec();
    for v in [0x20, total] {
        header.extend_from_slice(&u32::to_le_bytes(v));
    }
    header.extend_from_slice(&0u16.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    for v in [hash_pos, block_pos, HASH_COUNT as u32, blocks.len() as u32] {
        header.extend_from_slice(&v.to_le_bytes());
    }
    Parts {
        header,
        data,
        hash_plain,
        block_plain,
    }
}

/// A file in the temp directory, removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> TempFile {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "d2-formats-mpq-robust-{}-{n}.mpq",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("write temp archive");
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Opens `bytes` as an archive and runs every read path on it. Returns
/// whether the archive opened.
fn exercise_archive(bytes: Vec<u8>) -> bool {
    bounded(move || {
        let tmp = TempFile::new(&bytes);
        let Ok(a) = Archive::open(&tmp.0) else {
            return false;
        };
        for i in 0..a.block_table().len() {
            let _ = a.read_block(i, None);
            let _ = a.read_block(i, Some(0x1234_5678));
            if i < 16 {
                let _ = a.recover_key(i);
            }
        }
        for name in NAMES {
            let _ = a.contains(name);
            let _ = a.read(name);
            if let Some(i) = a.find(name) {
                let _ = a.read_block(i, a.file_key(name, i));
            }
        }
        let _ = a.listfile();
        true
    })
}

#[test]
fn synthetic_streams_decode() {
    let toks = [
        Tok::Lit(b'A'),
        Tok::Lit(b'I'),
        Tok::Copy { n: 11, back: 2 },
        Tok::Copy { n: 2, back: 13 },
    ];
    let out = explode::explode(&implode(4, &toks), 64).unwrap();
    assert_eq!(out, b"AIAIAIAIAIAIAAI");
    for channels in [1, 2] {
        let out = adpcm::decompress(&adpcm_stream(channels, 100), channels, 200);
        assert_eq!(out.len(), 200);
    }
}

#[test]
fn synthetic_archive_reads_back() {
    let bytes = synthetic_parts().assemble();
    let tmp = TempFile::new(&bytes);
    let a = Archive::open(&tmp.0).unwrap();
    for name in NAMES {
        assert_eq!(a.read(name).unwrap(), contents(name), "{name}");
    }
    let listed = a.listfile().unwrap().unwrap();
    assert_eq!(listed, NAMES[1..]);
    let b = a.find("b.bin").unwrap();
    assert_eq!(a.recover_key(b).unwrap(), a.file_key("b.bin", b));
    assert!(matches!(
        a.read_block(b, None),
        Err(MpqError::KeyRequired(_))
    ));
}

/// `parts` with block `index`'s plain block-table dword `field` (0 =
/// file_pos, 1 = compressed_size, 2 = file_size, 3 = flags) set to `v`.
fn with_block_field(parts: &Parts, index: usize, field: usize, v: u32) -> Vec<u8> {
    let mut p = parts.clone();
    let at = index * 16 + field * 4;
    p.block_plain[at..at + 4].copy_from_slice(&v.to_le_bytes());
    p.assemble()
}

#[test]
fn little_endian_and_wrapping_dwords() {
    let p = synthetic_parts();
    let bytes = p.assemble();
    let tmp = TempFile::new(&bytes);
    let a = Archive::open(&tmp.0).unwrap();
    let le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let h = a.header();
    assert_eq!(h.header_size, le(0x04));
    assert_eq!(h.archive_size, le(0x08));
    assert_eq!(
        h.sector_size_shift,
        u16::from_le_bytes([bytes[0x0E], bytes[0x0F]])
    );
    assert_eq!((h.hash_table_pos, h.block_table_pos), (le(0x10), le(0x14)));
    assert_eq!(
        (h.hash_table_count, h.block_table_count),
        (le(0x18), le(0x1C))
    );
    // One dword decrypted by hand with wrapping arithmetic and a key
    // near 2^32; the reader must agree (debug builds trap on overflow).
    let key = 0xFFFF_FFF0u32;
    let mut d = 0xDEAD_BEEFu32.to_le_bytes();
    let seed = 0xEEEE_EEEEu32.wrapping_add(crypto_table(0x400 + 0xF0));
    let want = 0xDEAD_BEEF ^ key.wrapping_add(seed);
    super::crypto::decrypt(&mut d, key);
    assert_eq!(u32::from_le_bytes(d), want);
    // A long name runs every hash seed through many wrapping additions.
    let long = "x\\".repeat(500);
    let _ = hash(long.as_bytes(), HashType::FileKey);
}

fn crypto_table(i: usize) -> u32 {
    super::crypto::CRYPT_TABLE[i]
}

#[test]
fn block_table_fields_and_flags() {
    use flags::*;
    let p = synthetic_parts();
    let tmp = TempFile::new(&p.assemble());
    let a = Archive::open(&tmp.0).unwrap();
    // Fields, decrypted with the block-table key, in table order.
    let want: Vec<BlockEntry> = p
        .block_plain
        .as_chunks::<16>()
        .0
        .iter()
        .map(|e| {
            let d = |i: usize| u32::from_le_bytes(e[i..i + 4].try_into().unwrap());
            BlockEntry {
                file_pos: d(0),
                compressed_size: d(4),
                file_size: d(8),
                flags: d(12),
            }
        })
        .collect();
    assert_eq!(a.block_table(), want);
    // Every flag combination of the synthetic archive reads back: IMPLODE
    // (no mask byte), COMPRESS (mask byte), ENCRYPTED, FIX_KEY,
    // SINGLE_UNIT, SECTOR_CRC.
    for name in NAMES {
        assert_eq!(a.read(name).unwrap(), contents(name), "{name}");
    }
    drop(a);

    let idx = |name: &str| NAMES.iter().position(|&n| n == name).unwrap();
    let flags_of = |name: &str| want[idx(name)].flags;
    let open = |bytes: Vec<u8>| {
        let tmp = TempFile::new(&bytes);
        let a = Archive::open(&tmp.0).unwrap();
        (tmp, a)
    };

    // PATCH_FILE is rejected.
    let (_t, a) = open(with_block_field(
        &p,
        idx("a.txt"),
        3,
        flags_of("a.txt") | PATCH_FILE,
    ));
    assert!(matches!(a.read("a.txt"), Err(MpqError::Unsupported(_))));
    // DELETE_MARKER: treated as not found.
    let (_t, a) = open(with_block_field(
        &p,
        idx("a.txt"),
        3,
        flags_of("a.txt") | DELETE_MARKER,
    ));
    assert!(matches!(a.read("a.txt"), Err(MpqError::NotFound(_))));
    // Without EXISTS, reading the block is an error.
    let (_t, a) = open(with_block_field(
        &p,
        idx("a.txt"),
        3,
        flags_of("a.txt") & !EXISTS,
    ));
    assert!(a.read("a.txt").is_err());
    // SECTOR_CRC adds one offset-table entry: without it the table is wrong.
    let (_t, a) = open(with_block_field(
        &p,
        idx("b.bin"),
        3,
        flags_of("b.bin") & !SECTOR_CRC,
    ));
    assert!(a.read("b.bin").is_err());
    // The data range must lie inside the file.
    let len = p.assemble().len() as u32;
    let (_t, a) = open(with_block_field(&p, idx("a.txt"), 1, len));
    assert!(a.read("a.txt").is_err());
    let (_t, a) = open(with_block_field(&p, idx("a.txt"), 0, len));
    assert!(a.read("a.txt").is_err());
}

#[test]
fn file_key_from_plain_name() {
    let p = synthetic_parts();
    let tmp = TempFile::new(&p.assemble());
    let a = Archive::open(&tmp.0).unwrap();
    let key = |name: &str| hash(name.as_bytes(), HashType::FileKey);

    // No FIX_KEY: the hash of the part after the last `\` or `/`.
    let d = a.find(r"sub\d.dat").unwrap();
    assert_eq!(a.file_key(r"sub\d.dat", d), Some(key("d.dat")));
    assert_eq!(a.file_key("x/y\\z/d.dat", d), Some(key("d.dat")));
    assert_eq!(
        a.read_block(d, Some(key("d.dat"))).unwrap(),
        contents(r"sub\d.dat")
    );
    let t = a.find("a.txt").unwrap();
    assert_eq!(a.file_key("a.txt", t), Some(key("a.txt")));

    // FIX_KEY: (base + file_pos) ^ file_size.
    let b = a.find("b.bin").unwrap();
    let e = a.block_table()[b];
    let fixed = key("b.bin").wrapping_add(e.file_pos) ^ e.file_size;
    assert_ne!(fixed, key("b.bin"));
    assert_eq!(a.file_key("b.bin", b), Some(fixed));
    assert_eq!(a.read_block(b, Some(fixed)).unwrap(), contents("b.bin"));
    assert!(a.read_block(b, Some(key("b.bin"))).is_err());

    // Not encrypted: no key.
    let c = a.find("c.wav").unwrap();
    assert_eq!(a.file_key("c.wav", c), None);
}

#[test]
fn format_edge_cases() {
    // Encrypted lengths not a multiple of 4: the last 1–3 bytes stay plain.
    for tail in 1..4 {
        let plain: Vec<u8> = (0..8 + tail as u8).collect();
        let mut enc = plain.clone();
        encrypt(&mut enc, 0x1234_5678);
        assert_eq!(enc[8..], plain[8..], "tail {tail}");
        assert_ne!(enc[..8], plain[..8]);
        let mut whole = plain[..8].to_vec();
        encrypt(&mut whole, 0x1234_5678);
        assert_eq!(enc[..8], whole[..]);
        super::crypto::decrypt(&mut enc, 0x1234_5678);
        assert_eq!(enc, plain);
    }

    // A sector exactly as long as its decoded size is stored raw under
    // COMPRESS: b.bin's last sector is 276 raw bytes whose first byte (0x0B)
    // would be an invalid mask if decoded.
    let p = synthetic_parts();
    let tmp = TempFile::new(&p.assemble());
    let a = Archive::open(&tmp.0).unwrap();
    let c = contents("b.bin");
    assert_eq!(c[2 * SECTOR], 0x0B);
    assert!(decompress_masked(c[2 * SECTOR], &c[2 * SECTOR + 1..], c.len() - 2 * SECTOR).is_err());
    assert_eq!(a.read("b.bin").unwrap(), c);

    // The PKWARE end marker may end exactly at the end of input: 8 literals
    // (72 bits) + end marker (1 + 7 + 8 bits) = 11 bytes, no padding.
    let s = implode(4, &literals(b"abcdefgh"));
    assert_eq!(s.len(), 2 + 11);
    assert_eq!(explode::explode(&s, 64).unwrap(), b"abcdefgh");

    // Even under tables 1–8 (not the adaptive table 0), an escaped value
    // gets two weight increments: the decoder agrees with the reference
    // model only when the model applies the second increment.
    let data = b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    let mut differs = false;
    for t in 1..9u8 {
        let mut d = data.to_vec();
        d.extend(b"\x10\x10\x10\x10\x10\x11\x11\x11\x12\x12");
        let stream = huffman::compress(t, &d);
        let got = huffman::decompress(&stream, 4096).unwrap();
        assert_eq!(got, d);
        assert_eq!(Ok(got.clone()), Model::decode(&stream, 4096, 2));
        differs |= Model::decode(&stream, 4096, 1) != Ok(got);
    }
    assert!(differs, "one increment would decode differently");
}

// ---------------------------------------------------------------------------
// Minimized inputs for allocation limits.

/// Decoders never reserve their whole output cap up front: a cap of
/// `usize::MAX` still decodes normally, and a sector claiming a 4 GiB file
/// fails cleanly.
#[test]
fn regress_decoder_capacity_from_max_out() {
    let blast = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F];
    assert_eq!(
        explode::explode(&blast, usize::MAX).unwrap(),
        b"AIAIAIAIAIAIA"
    );
    let h = huffman::compress(0, b"abc");
    assert_eq!(huffman::decompress(&h, usize::MAX).unwrap(), b"abc");
    assert_eq!(
        adpcm::decompress(&[0x00, 0x00, 0x10, 0x00, 0x80], 1, usize::MAX),
        [16, 0, 16, 0]
    );
    let block = BlockEntry {
        file_pos: 0,
        compressed_size: 0,
        file_size: u32::MAX,
        flags: flags::EXISTS | flags::COMPRESS,
    };
    assert!(decompress_sector(&block, &masked(0x08, &blast), u32::MAX as usize, None).is_err());
}

/// A 4 GiB `file_size` on a tiny block: single-unit and sectored reads fail
/// cleanly instead of reserving the whole size before decoding anything.
#[test]
fn regress_huge_file_size() {
    for (f, shift) in [
        (flags::EXISTS | flags::COMPRESS | flags::SINGLE_UNIT, 0u16),
        (flags::EXISTS | flags::COMPRESS, 15),
        (flags::EXISTS | flags::IMPLODE, 15),
    ] {
        let mut p = synthetic_parts();
        p.header[0x0E..0x10].copy_from_slice(&shift.to_le_bytes());
        // Block 1 (a.txt): keep its data, claim a 4 GiB file.
        p.block_plain[16 + 8..16 + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        p.block_plain[16 + 12..16 + 16].copy_from_slice(&f.to_le_bytes());
        let tmp = TempFile::new(&p.assemble());
        let a = Archive::open(&tmp.0).unwrap();
        assert!(a.read_block(1, None).is_err());
    }
}

// ---------------------------------------------------------------------------
// Properties.

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn explode_arbitrary(input in bytes(600), max in max_out()) {
        bounded(move || explode::explode(&input, max).map(|v| v.len()).ok());
    }

    #[test]
    fn explode_arbitrary_header(dict in 4u8..=6, ascii in 0u8..2, body in bytes(600), max in max_out()) {
        let mut input = vec![ascii, dict];
        input.extend_from_slice(&body);
        bounded(move || explode::explode(&input, max).map(|v| v.len()).ok());
    }

    #[test]
    fn huffman_arbitrary(table in 0u8..9, body in bytes(600), max in max_out()) {
        let mut input = vec![table];
        input.extend_from_slice(&body);
        bounded(move || huffman::decompress(&input, max).map(|v| v.len()).ok());
    }

    #[test]
    fn adpcm_arbitrary(input in bytes(600), channels in 1usize..=2, max in max_out()) {
        bounded(move || adpcm::decompress(&input, channels, max).len());
    }

    #[test]
    fn dispatcher_arbitrary(mask in any::<u8>(), payload in bytes(600), expected in max_out()) {
        bounded(move || decompress_masked(mask, &payload, expected).map(|v| v.len()).ok());
    }

    #[test]
    fn sector_arbitrary(implode_flag in any::<bool>(), sector in bytes(600), expected in max_out()) {
        let block = BlockEntry {
            file_pos: 0,
            compressed_size: 0,
            file_size: 0,
            flags: flags::EXISTS | if implode_flag { flags::IMPLODE } else { flags::COMPRESS },
        };
        bounded(move || decompress_sector(&block, &sector, expected, None).map(|v| v.len()).ok());
    }

    #[test]
    fn explode_mutated(
        input in mutated(implode(5, &[
            Tok::Lit(b'q'), Tok::Lit(b'r'), Tok::Copy { n: 40, back: 2 },
            Tok::Lit(b's'), Tok::Copy { n: 2, back: 3 }, Tok::Copy { n: 300, back: 7 },
        ])),
        max in max_out(),
    ) {
        bounded(move || explode::explode(&input, max).map(|v| v.len()).ok());
    }

    #[test]
    fn huffman_mutated(
        input in (0u8..9).prop_flat_map(|t| mutated(huffman::compress(t, &text(300)))),
        max in max_out(),
    ) {
        bounded(move || huffman::decompress(&input, max).map(|v| v.len()).ok());
    }

    #[test]
    fn adpcm_mutated(
        channels in 1usize..=2,
        input in prop_oneof![mutated(adpcm_stream(1, 200)), mutated(adpcm_stream(2, 200))],
        max in max_out(),
    ) {
        bounded(move || adpcm::decompress(&input, channels, max).len());
    }

    #[test]
    fn dispatcher_mutated(
        sector in prop_oneof![
            mutated(masked(0x09, &implode(5, &literals(&huffman::compress(0, &text(200)))))),
            mutated(masked(0x41, &huffman::compress(0, &adpcm_stream(1, 100)))),
            mutated(masked(0x81, &huffman::compress(2, &adpcm_stream(2, 100)))),
        ],
        expected in max_out(),
    ) {
        bounded(move || {
            let (&mask, payload) = sector.split_first()?;
            decompress_masked(mask, payload, expected).map(|v| v.len()).ok()
        });
    }
}

proptest! {
    #![proptest_config(config(48))]

    #[test]
    fn archive_arbitrary(mut input in bytes(256), header in any::<bool>()) {
        if header {
            input.splice(0..0, b"MPQ\x1A".iter().copied());
        }
        exercise_archive(input);
    }

    #[test]
    fn archive_mutated(input in mutated(synthetic_parts().assemble())) {
        exercise_archive(input);
    }

    #[test]
    fn archive_mutated_plain_tables(
        input in {
            let p = synthetic_parts();
            let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
            prop_oneof![
                mutated(p.header.clone()).prop_map(move |header| Parts { header, ..p1.clone() }.assemble()),
                mutated(p.block_plain.clone())
                    .prop_map(move |block_plain| Parts { block_plain, ..p2.clone() }.assemble()),
                mutated(p.hash_plain.clone())
                    .prop_map(move |hash_plain| Parts { hash_plain, ..p3.clone() }.assemble()),
            ]
        }
    ) {
        exercise_archive(input);
    }
}

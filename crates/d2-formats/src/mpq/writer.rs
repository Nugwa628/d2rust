//! A test-only MPQ writer: builds format-version-0 archives that
//! [`super::Archive`] reads, so the load path can be tested without game
//! files. Not used by the game: D2RUST only reads archives, and this module exists
//! only behind the `test-support` feature (and for this crate's own tests).
//!
//! Layout written: the 32-byte header at offset 0, the file data in the
//! order files were added, then the hash table, then the block table, both
//! encrypted with their fixed keys. Hash slots are placed by the same
//! probe the reader uses, so lookups find every file; `(listfile)` is added on request.
//!
//! Compression: the PKWARE DCL stream, either under the IMPLODE
//! flag or under COMPRESS with mask 0x08, literal mode binary or ASCII,
//! dictionary bits 4–6; the Huffman stream of any weight table under
//! COMPRESS with mask 0x01, or Huffman then PKWARE with mask 0x09. A sector
//! that does not shrink is stored raw, which the reader treats as stored.
//! Encryption with or without FIX_KEY; SINGLE_UNIT and SECTOR_CRC layouts.

use std::path::Path;

use super::crypto::{encrypt, hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
use super::flags;
use super::tables::{
    CH_BITS, CH_CODE, DIST_BITS, DIST_CODE, EX_LEN_BITS, LEN_BASE, LEN_BITS, LEN_CODE,
};
use super::{compression, huffman, HEADER_MAGIC};

/// How the sectors of one file are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Neither IMPLODE nor COMPRESS.
    Stored,
    /// The IMPLODE flag: each sector a bare PKWARE stream.
    Implode(Pkware),
    /// The COMPRESS flag: each compressed sector is mask 0x08 + a PKWARE
    /// stream.
    Compress(Pkware),
    /// The COMPRESS flag with a Huffman stage: each compressed
    /// sector is mask 0x01 + a Huffman stream with weight table `table`
    /// (0..=8), or with `pkware`, mask 0x09 + a PKWARE stream of that
    /// Huffman stream (the reader explodes first, then decodes Huffman).
    Huffman {
        /// Huffman weight table, 0..=8.
        table: u8,
        /// If set, the Huffman stream is further wrapped in a PKWARE stream.
        pkware: Option<Pkware>,
    },
}

/// PKWARE DCL stream parameters (the two header bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pkware {
    /// Literal mode 1 (ASCII codes) instead of 0 (binary).
    pub ascii: bool,
    /// Dictionary bits `D`, 4–6.
    pub dict_bits: u8,
}

impl Default for Pkware {
    fn default() -> Self {
        Pkware {
            ascii: false,
            dict_bits: 6,
        }
    }
}

/// Per-file options. The default is a compressed (COMPRESS + PKWARE),
/// unencrypted, sectored file with locale 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOptions {
    /// How sectors are stored.
    pub method: Method,
    /// Encrypt the file data.
    pub encrypted: bool,
    /// FIX_KEY (only meaningful with `encrypted`).
    pub fix_key: bool,
    /// SINGLE_UNIT: the file is one block, not sectors.
    pub single_unit: bool,
    /// SECTOR_CRC: one extra offset entry and a (zero) CRC block, which
    /// the reader ignores. Only for sectored compressed files.
    pub sector_crc: bool,
    /// Locale id written to the file's hash entry.
    pub locale: u16,
}

impl Default for FileOptions {
    fn default() -> Self {
        FileOptions {
            method: Method::Compress(Pkware::default()),
            encrypted: false,
            fix_key: false,
            single_unit: false,
            sector_crc: false,
            locale: 0,
        }
    }
}

impl FileOptions {
    /// Stored (uncompressed, unencrypted) with the other defaults.
    pub fn stored() -> Self {
        FileOptions {
            method: Method::Stored,
            ..FileOptions::default()
        }
    }

    /// COMPRESS + ENCRYPTED + FIX_KEY, the flag layout of the game's `.wav`
    /// files, with PKWARE instead of Huffman + ADPCM.
    pub fn encrypted_fix_key() -> Self {
        FileOptions {
            encrypted: true,
            fix_key: true,
            ..FileOptions::default()
        }
    }
}

#[derive(Debug, Clone)]
enum Entry {
    File {
        name: String,
        data: Vec<u8>,
        options: FileOptions,
    },
    /// A hash entry marked deleted (0xFFFFFFFE), placed in the probe
    /// chain of `name`; it has no block.
    Deleted { name: String },
}

/// Builds an archive in memory.
#[derive(Debug, Clone)]
pub struct MpqWriter {
    sector_size_shift: u16,
    hash_table_count: Option<u32>,
    listfile: bool,
    entries: Vec<Entry>,
}

impl Default for MpqWriter {
    fn default() -> Self {
        MpqWriter::new()
    }
}

/// The archive could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// More hash entries than the table has slots.
    #[error("hash table of {0} entries is full")]
    HashTableFull(u32),
    /// The requested hash table size is not a power of two.
    #[error("hash table count {0} is not a power of two")]
    BadHashCount(u32),
    /// Offsets would not fit in 32 bits.
    #[error("archive larger than 4 GiB")]
    TooLarge,
    /// PKWARE dictionary bits outside 4..=6.
    #[error("dictionary bits {0} outside 4..=6")]
    BadDictBits(u8),
    /// Huffman weight table index outside 0..=8.
    #[error("Huffman table {0} outside 0..=8")]
    BadHuffmanTable(u8),
    /// SECTOR_CRC was requested for a file that is not sectored and compressed.
    #[error("{0}: SECTOR_CRC needs a sectored, compressed file")]
    BadCrc(String),
}

impl MpqWriter {
    /// Sector size 4096 (shift 3), the sector size the game's archives use.
    pub fn new() -> Self {
        MpqWriter {
            sector_size_shift: 3,
            hash_table_count: None,
            listfile: false,
            entries: Vec::new(),
        }
    }

    /// Sector size `0x200 << shift`.
    pub fn sector_size_shift(mut self, shift: u16) -> Self {
        self.sector_size_shift = shift;
        self
    }

    /// Fixed hash table size (a power of two). Default: the smallest power
    /// of two ≥ 16 and ≥ twice the number of hash entries.
    pub fn hash_table_count(mut self, count: u32) -> Self {
        self.hash_table_count = Some(count);
        self
    }

    /// Adds a `(listfile)` naming every added file.
    pub fn with_listfile(mut self) -> Self {
        self.listfile = true;
        self
    }

    /// Adds a file with explicit options.
    pub fn add(&mut self, name: &str, data: impl Into<Vec<u8>>, options: FileOptions) {
        self.entries.push(Entry::File {
            name: name.to_owned(),
            data: data.into(),
            options,
        });
    }

    /// Adds a file with the default options.
    pub fn add_file(&mut self, name: &str, data: impl Into<Vec<u8>>) {
        self.add(name, data, FileOptions::default());
    }

    /// Adds a deleted hash entry in `name`'s probe chain.
    pub fn add_deleted(&mut self, name: &str) {
        self.entries.push(Entry::Deleted {
            name: name.to_owned(),
        });
    }

    fn sector_size(&self) -> usize {
        0x200usize << self.sector_size_shift
    }

    /// The archive bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, WriteError> {
        let mut entries = self.entries.clone();
        if self.listfile {
            let mut list = Vec::new();
            for e in &self.entries {
                if let Entry::File { name, .. } = e {
                    list.extend_from_slice(name.as_bytes());
                    list.extend_from_slice(b"\r\n");
                }
            }
            entries.push(Entry::File {
                name: "(listfile)".to_owned(),
                data: list,
                options: FileOptions::default(),
            });
        }
        let hash_count = match self.hash_table_count {
            Some(c) if !c.is_power_of_two() => return Err(WriteError::BadHashCount(c)),
            Some(c) => c,
            None => (entries.len() as u32 * 2).max(16).next_power_of_two(),
        };

        let mut out = vec![0u8; 0x20];
        let mut blocks: Vec<[u32; 4]> = Vec::new();
        // (name, locale, block index or deleted)
        let mut hashed: Vec<(&str, u16, u32)> = Vec::new();
        for e in &entries {
            match e {
                Entry::Deleted { name } => hashed.push((name, 0, 0xFFFF_FFFE)),
                Entry::File {
                    name,
                    data,
                    options,
                } => {
                    let pos = u32::try_from(out.len()).map_err(|_| WriteError::TooLarge)?;
                    let (bytes, flags) = self.encode_file(name, data, options, pos)?;
                    let size = u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
                    out.extend_from_slice(&bytes);
                    hashed.push((name, options.locale, blocks.len() as u32));
                    blocks.push([pos, size, data.len() as u32, flags]);
                }
            }
        }

        let mut table = vec![[0xFFFF_FFFFu32; 4]; hash_count as usize];
        for &(name, locale, block) in &hashed {
            let n = name.as_bytes();
            let mut slot = hash(n, HashType::TableOffset) & (hash_count - 1);
            let mut tries = 0;
            while table[slot as usize][3] != 0xFFFF_FFFF {
                slot = (slot + 1) & (hash_count - 1);
                tries += 1;
                if tries == hash_count {
                    return Err(WriteError::HashTableFull(hash_count));
                }
            }
            table[slot as usize] = [
                hash(n, HashType::NameA),
                hash(n, HashType::NameB),
                u32::from(locale),
                block,
            ];
        }

        let hash_pos = u32::try_from(out.len()).map_err(|_| WriteError::TooLarge)?;
        let mut bytes: Vec<u8> = table
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        encrypt(&mut bytes, HASH_TABLE_KEY);
        out.extend_from_slice(&bytes);
        let block_pos = u32::try_from(out.len()).map_err(|_| WriteError::TooLarge)?;
        let mut bytes: Vec<u8> = blocks
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        encrypt(&mut bytes, BLOCK_TABLE_KEY);
        out.extend_from_slice(&bytes);
        let archive_size = u32::try_from(out.len()).map_err(|_| WriteError::TooLarge)?;

        out[0..4].copy_from_slice(&HEADER_MAGIC);
        out[0x04..0x08].copy_from_slice(&0x20u32.to_le_bytes());
        out[0x08..0x0C].copy_from_slice(&archive_size.to_le_bytes());
        out[0x0C..0x0E].copy_from_slice(&0u16.to_le_bytes());
        out[0x0E..0x10].copy_from_slice(&self.sector_size_shift.to_le_bytes());
        out[0x10..0x14].copy_from_slice(&hash_pos.to_le_bytes());
        out[0x14..0x18].copy_from_slice(&block_pos.to_le_bytes());
        out[0x18..0x1C].copy_from_slice(&hash_count.to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
        Ok(out)
    }

    /// Writes the archive to `path`.
    pub fn write(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        std::fs::write(path, bytes)
    }

    /// The stored bytes and block flags of one file at archive offset `pos`.
    fn encode_file(
        &self,
        name: &str,
        data: &[u8],
        o: &FileOptions,
        pos: u32,
    ) -> Result<(Vec<u8>, u32), WriteError> {
        let mut fl = flags::EXISTS;
        let pkware = match o.method {
            Method::Stored => None,
            Method::Implode(p) => {
                fl |= flags::IMPLODE;
                Some(p)
            }
            Method::Compress(p) => {
                fl |= flags::COMPRESS;
                Some(p)
            }
            Method::Huffman { table, pkware } => {
                if usize::from(table) >= super::tables::HUFFMAN_WEIGHTS.len() {
                    return Err(WriteError::BadHuffmanTable(table));
                }
                fl |= flags::COMPRESS;
                pkware
            }
        };
        if let Some(p) = pkware {
            if !(4..=6).contains(&p.dict_bits) {
                return Err(WriteError::BadDictBits(p.dict_bits));
            }
        }
        let stored = o.method == Method::Stored;
        if o.single_unit {
            fl |= flags::SINGLE_UNIT;
        }
        if o.sector_crc {
            if stored || o.single_unit {
                return Err(WriteError::BadCrc(name.to_owned()));
            }
            fl |= flags::SECTOR_CRC;
        }
        let key = if o.encrypted {
            fl |= flags::ENCRYPTED;
            let plain = name.rsplit(['\\', '/']).next().unwrap_or(name);
            let base = hash(plain.as_bytes(), HashType::FileKey);
            if o.fix_key {
                fl |= flags::FIX_KEY;
                base.wrapping_add(pos) ^ data.len() as u32
            } else {
                base
            }
        } else {
            0
        };
        if data.is_empty() {
            return Ok((Vec::new(), fl));
        }

        let compress = |unit: &[u8]| -> Vec<u8> {
            let c = match o.method {
                Method::Stored => return unit.to_vec(),
                Method::Implode(p) => implode(unit, p),
                Method::Compress(p) => masked(compression::PKWARE, &implode(unit, p)),
                Method::Huffman { table, pkware } => {
                    let h = huffman::compress(table, unit);
                    match pkware {
                        None => masked(compression::HUFFMAN, &h),
                        // Each decoder's output is capped at the sector
                        // size, so the Huffman stream between the two
                        // stages must fit in it.
                        Some(p) if h.len() <= unit.len() => {
                            masked(compression::HUFFMAN | compression::PKWARE, &implode(&h, p))
                        }
                        Some(_) => return unit.to_vec(),
                    }
                }
            };
            if c.len() < unit.len() {
                c
            } else {
                unit.to_vec()
            }
        };

        if o.single_unit {
            let mut unit = compress(data);
            if o.encrypted {
                encrypt(&mut unit, key);
            }
            return Ok((unit, fl));
        }

        let s = self.sector_size();
        if stored {
            let mut out = data.to_vec();
            if o.encrypted {
                for (i, chunk) in out.chunks_mut(s).enumerate() {
                    encrypt(chunk, key.wrapping_add(i as u32));
                }
            }
            return Ok((out, fl));
        }

        let sectors: Vec<Vec<u8>> = data
            .chunks(s)
            .enumerate()
            .map(|(i, chunk)| {
                let mut c = compress(chunk);
                if o.encrypted {
                    encrypt(&mut c, key.wrapping_add(i as u32));
                }
                c
            })
            .collect();
        let entries = sectors.len() + 1 + usize::from(o.sector_crc);
        let mut offsets = Vec::with_capacity(entries);
        let mut at = entries * 4;
        offsets.push(at as u32);
        for sector in &sectors {
            at += sector.len();
            offsets.push(at as u32);
        }
        let crc_block = if o.sector_crc {
            // The CRC block is ignored by the reader; zeros.
            let b = vec![0u8; sectors.len() * 4];
            offsets.push((at + b.len()) as u32);
            b
        } else {
            Vec::new()
        };
        let mut table: Vec<u8> = offsets.iter().flat_map(|v| v.to_le_bytes()).collect();
        if o.encrypted {
            encrypt(&mut table, key.wrapping_sub(1));
        }
        let mut out = table;
        for sector in sectors {
            out.extend_from_slice(&sector);
        }
        out.extend_from_slice(&crc_block);
        Ok((out, fl))
    }
}

/// A COMPRESS sector: the mask byte, then the payload.
fn masked(mask: u8, payload: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(payload.len() + 1);
    c.push(mask);
    c.extend_from_slice(payload);
    c
}

/// A Huffman stream with weight table `table` (0..=8) that the
/// reader's Huffman stage decodes to `data`; it ends with the end symbol.
/// Panics on a table outside 0..=8.
pub fn huffman(table: u8, data: &[u8]) -> Vec<u8> {
    huffman::compress(table, data)
}

/// LSB-first bit writer.
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    count: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, bits: u32) {
        debug_assert!(bits <= 32);
        self.acc |= u64::from(value & ((1u64 << bits) - 1) as u32) << self.count;
        self.count += bits;
        while self.count >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.count -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Longest copy a PKWARE stream can encode.
const MAX_COPY: usize = 518;
/// How many earlier positions with the same 3-byte prefix are tried.
const CHAIN_LIMIT: usize = 64;

/// Writes the length `l` (0..=0x205) as a length symbol and extra bits.
fn put_length(w: &mut BitWriter, l: u32) {
    let sym = (0..16)
        .rev()
        .find(|&s| {
            let base = if EX_LEN_BITS[s] == 0 {
                s as u32
            } else {
                u32::from(LEN_BASE[s])
            };
            l >= base
        })
        .expect("length symbol 0 covers 0");
    w.put(u32::from(LEN_CODE[sym]), u32::from(LEN_BITS[sym]));
    let extra = u32::from(EX_LEN_BITS[sym]);
    if extra > 0 {
        w.put(l - u32::from(LEN_BASE[sym]), extra);
    }
}

/// A PKWARE DCL stream that [`super::explode`] decodes to `data`:
/// greedy matches over a hash chain, the end marker last.
pub fn implode(data: &[u8], p: Pkware) -> Vec<u8> {
    let d = u32::from(p.dict_bits);
    let max_dist = 64usize << d; // largest distance + 1 for n > 2
    let mut w = BitWriter {
        out: vec![u8::from(p.ascii), p.dict_bits],
        acc: 0,
        count: 0,
    };
    let key = |i: usize| -> usize {
        (usize::from(data[i]) << 16 | usize::from(data[i + 1]) << 8 | usize::from(data[i + 2]))
            % 4093
    };
    let mut head = vec![usize::MAX; 4093];
    let mut prev = vec![usize::MAX; data.len()];
    let insert = |i: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if i + 3 <= data.len() {
            let k = key(i);
            prev[i] = head[k];
            head[k] = i;
        }
    };

    let mut i = 0;
    while i < data.len() {
        let mut best = (0usize, 0usize); // (len, back)
        if i + 3 <= data.len() {
            let mut cand = head[key(i)];
            let mut tries = 0;
            while cand != usize::MAX && tries < CHAIN_LIMIT && i - cand <= max_dist {
                let limit = (data.len() - i).min(MAX_COPY);
                let mut n = 0;
                while n < limit && data[cand + n] == data[i + n] {
                    n += 1;
                }
                if n > best.0 {
                    best = (n, i - cand);
                }
                cand = prev[cand];
                tries += 1;
            }
        }
        // A 2-byte copy needs distance < 256 (2 extra bits); take it only
        // from the immediate window.
        if best.0 < 3 && i + 2 <= data.len() {
            best = (0, 0);
            for back in 1..=i.min(256) {
                if data[i - back] == data[i] && data[i - back + 1] == data[i + 1] {
                    best = (2, back);
                    break;
                }
            }
        }
        if best.0 >= 2 {
            let (n, back) = best;
            w.put(1, 1);
            put_length(&mut w, (n - 2) as u32);
            let dist = (back - 1) as u32;
            let (sym, r, rbits) = if n == 2 {
                (dist >> 2, dist & 3, 2)
            } else {
                (dist >> d, dist & ((1 << d) - 1), d)
            };
            w.put(
                u32::from(DIST_CODE[sym as usize]),
                u32::from(DIST_BITS[sym as usize]),
            );
            w.put(r, rbits);
            for k in i..i + n {
                insert(k, &mut head, &mut prev);
            }
            i += n;
        } else {
            w.put(0, 1);
            let b = data[i];
            if p.ascii {
                w.put(
                    u32::from(CH_CODE[usize::from(b)]),
                    u32::from(CH_BITS[usize::from(b)]),
                );
            } else {
                w.put(u32::from(b), 8);
            }
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    // End marker: a copy flag and length 0x205.
    w.put(1, 1);
    put_length(&mut w, 0x205);
    w.finish()
}

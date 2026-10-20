//! Reading one MPQ archive: finding its header, loading its hash and block tables, and
//! turning a file name into the file's bytes.
//!
//! A file is located by hashing its name into the hash table, which points at an entry of
//! the block table. That entry says where the stored bytes are and how they are packed:
//! as one unit or as fixed-size sectors, encrypted or not, and compressed or not. The
//! per-sector codecs live in `sector`; the cipher and the name hash in `crypto`.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use super::crypto::{decrypt, hash, HashType, BLOCK_TABLE_KEY, CRYPT_TABLE, HASH_TABLE_KEY};
use super::sector::decompress_sector;
use super::{
    flags, BlockEntry, CodecError, HashEntry, Header, MpqError, SectorStats, HEADER_MAGIC,
    USER_DATA_MAGIC,
};

/// An open MPQ archive. Reads use positional I/O, so `&Archive` can be
/// shared between threads.
#[derive(Debug)]
pub struct Archive {
    path: PathBuf,
    file: File,
    file_len: u64,
    header: Header,
    hash_table: Vec<HashEntry>,
    block_table: Vec<BlockEntry>,
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

fn corrupt(msg: impl Into<String>) -> MpqError {
    MpqError::Corrupt(msg.into())
}

impl Archive {
    /// Opens an archive and reads its header and tables.
    pub fn open(path: impl AsRef<Path>) -> Result<Archive, MpqError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let file_len = file.metadata()?.len();
        let header = Self::find_header(&file, file_len)?;

        let hash_bytes = Self::read_table(
            &file,
            file_len,
            header.offset + u64::from(header.hash_table_pos),
            header.hash_table_count,
            HASH_TABLE_KEY,
            "hash table",
        )?;
        let hash_table = hash_bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|e| HashEntry {
                name_a: le_u32(e, 0),
                name_b: le_u32(e, 4),
                locale: le_u16(e, 8),
                platform: le_u16(e, 10),
                block_index: le_u32(e, 12),
            })
            .collect();

        let block_bytes = Self::read_table(
            &file,
            file_len,
            header.offset + u64::from(header.block_table_pos),
            header.block_table_count,
            BLOCK_TABLE_KEY,
            "block table",
        )?;
        let block_table = block_bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|e| BlockEntry {
                file_pos: le_u32(e, 0),
                compressed_size: le_u32(e, 4),
                file_size: le_u32(e, 8),
                flags: le_u32(e, 12),
            })
            .collect();

        Ok(Archive {
            path,
            file,
            file_len,
            header,
            hash_table,
            block_table,
        })
    }

    /// Finds and validates the header. An archive may sit inside a larger file (after an
    /// installer stub, say), so the header is searched for at every 0x200-byte boundary,
    /// through the whole file. The first "MPQ\x1A" found is the header; a user-data
    /// header ("MPQ\x1B") found first points at the real header instead.
    fn find_header(file: &File, file_len: u64) -> Result<Header, MpqError> {
        const CHUNK: u64 = 1 << 20;
        let mut chunk = vec![0u8; CHUNK.min(file_len) as usize];
        let mut base = 0u64;
        while base + 32 <= file_len {
            let len = CHUNK.min(file_len - base) as usize;
            read_exact_at(file, &mut chunk[..len], base)?;
            let mut at = 0usize;
            while at + 4 <= len {
                let magic = &chunk[at..at + 4];
                let candidate = base + at as u64;
                if magic == HEADER_MAGIC {
                    return Self::parse_header(file, file_len, candidate);
                }
                if magic == USER_DATA_MAGIC && at + 12 <= len {
                    let header_offset = u64::from(le_u32(&chunk, at + 8));
                    return Self::parse_header(file, file_len, candidate + header_offset);
                }
                at += 0x200;
            }
            base += CHUNK;
        }
        Err(MpqError::NotAnArchive)
    }

    /// Reads the 32-byte header at `offset` and rejects the values the reader can't
    /// work with: a short header, a format version other than 0, an absurd sector size,
    /// or a hash table whose size isn't a power of two (lookups mask with `count - 1`).
    fn parse_header(file: &File, file_len: u64, offset: u64) -> Result<Header, MpqError> {
        if offset + 32 > file_len {
            return Err(MpqError::NotAnArchive);
        }
        let mut h = [0u8; 32];
        read_exact_at(file, &mut h, offset)?;
        if h[0..4] != HEADER_MAGIC {
            return Err(MpqError::NotAnArchive);
        }
        let header = Header {
            offset,
            header_size: le_u32(&h, 0x04),
            archive_size: le_u32(&h, 0x08),
            format_version: le_u16(&h, 0x0C),
            sector_size_shift: le_u16(&h, 0x0E),
            hash_table_pos: le_u32(&h, 0x10),
            block_table_pos: le_u32(&h, 0x14),
            hash_table_count: le_u32(&h, 0x18),
            block_table_count: le_u32(&h, 0x1C),
        };
        if header.header_size < 0x20 {
            return Err(corrupt(format!("header size {:#x}", header.header_size)));
        }
        if header.format_version != 0 {
            return Err(MpqError::UnsupportedVersion(header.format_version));
        }
        if header.sector_size_shift > 15 {
            return Err(corrupt(format!(
                "sector size shift {}",
                header.sector_size_shift
            )));
        }
        if !header.hash_table_count.is_power_of_two() {
            return Err(corrupt(format!(
                "hash table count {} is not a power of two",
                header.hash_table_count
            )));
        }
        Ok(header)
    }

    /// Reads and decrypts one of the two tables (16 bytes per entry).
    fn read_table(
        file: &File,
        file_len: u64,
        pos: u64,
        count: u32,
        key: u32,
        what: &str,
    ) -> Result<Vec<u8>, MpqError> {
        let len = u64::from(count) * 16;
        if pos.checked_add(len).is_none_or(|end| end > file_len) {
            return Err(corrupt(format!("{what} extends past end of file")));
        }
        let mut bytes = vec![0u8; len as usize];
        read_exact_at(file, &mut bytes, pos)?;
        decrypt(&mut bytes, key);
        Ok(bytes)
    }

    /// Path the archive was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The parsed header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The decrypted hash table.
    pub fn hash_table(&self) -> &[HashEntry] {
        &self.hash_table
    }

    /// The decrypted block table.
    pub fn block_table(&self) -> &[BlockEntry] {
        &self.block_table
    }

    /// Sector size in bytes.
    pub fn sector_size(&self) -> u32 {
        0x200 << self.header.sector_size_shift
    }

    /// Looks up `name` in the hash table and returns its block index.
    ///
    /// The name's table-offset hash picks the first slot; probing then walks forward
    /// (wrapping) until a never-used slot ends the chain. A slot matches when both name
    /// check hashes agree and its block index is in range. Only the neutral-locale,
    /// default-platform copy of a file is ever chosen, and the first such match wins.
    pub fn find(&self, name: &str) -> Option<usize> {
        let bytes = name.as_bytes();
        let count = self.hash_table.len();
        let start = hash(bytes, HashType::TableOffset) as usize & (count - 1);
        let a = hash(bytes, HashType::NameA);
        let b = hash(bytes, HashType::NameB);

        for i in 0..count {
            let e = &self.hash_table[(start + i) & (count - 1)];
            if e.is_empty() {
                break;
            }
            if e.is_deleted() {
                continue;
            }
            if e.name_a == a
                && e.name_b == b
                && (e.block_index as usize) < self.block_table.len()
                && e.locale == 0
                && e.platform == 0
            {
                return Some(e.block_index as usize);
            }
        }
        None
    }

    /// True if `name` is present and not a delete marker.
    pub fn contains(&self, name: &str) -> bool {
        self.find(name)
            .is_some_and(|i| !self.block_table[i].has(flags::DELETE_MARKER))
    }

    /// Reads a file by name.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, MpqError> {
        let index = self
            .find(name)
            .ok_or_else(|| MpqError::NotFound(name.to_owned()))?;
        if self.block_table[index].has(flags::DELETE_MARKER) {
            return Err(MpqError::NotFound(name.to_owned()));
        }
        let key = self.file_key(name, index);
        self.read_block(index, key)
    }

    /// The encryption key for `name` stored in block `index`, or
    /// `None` if the block isn't encrypted.
    ///
    /// The key is the hash of the bare file name (the part after the last `\` or `/`).
    /// With [`flags::FIX_KEY`] it is also tied to the block's position and size, so
    /// the same file stored twice encrypts differently.
    pub fn file_key(&self, name: &str, index: usize) -> Option<u32> {
        let block = self.block_table.get(index)?;
        if !block.has(flags::ENCRYPTED) {
            return None;
        }
        let plain = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let base = hash(plain.as_bytes(), HashType::FileKey);
        Some(if block.has(flags::FIX_KEY) {
            base.wrapping_add(block.file_pos) ^ block.file_size
        } else {
            base
        })
    }

    /// Reads block `index`. `key` is required if the block is encrypted.
    pub fn read_block(&self, index: usize, key: Option<u32>) -> Result<Vec<u8>, MpqError> {
        self.read_block_impl(index, key, None)
    }

    /// Like [`Archive::read_block`], also counting compression masks.
    pub fn read_block_stats(
        &self,
        index: usize,
        key: Option<u32>,
        stats: &mut SectorStats,
    ) -> Result<Vec<u8>, MpqError> {
        self.read_block_impl(index, key, Some(stats))
    }

    /// The stored bytes of `block`, exactly as they are on disk.
    fn read_raw(&self, block: &BlockEntry, index: usize) -> Result<Vec<u8>, MpqError> {
        let start = self.header.offset + u64::from(block.file_pos);
        let len = u64::from(block.compressed_size);
        if start + len > self.file_len {
            return Err(corrupt(format!("block {index} extends past end of file")));
        }
        let mut data = vec![0u8; len as usize];
        read_exact_at(&self.file, &mut data, start)?;
        Ok(data)
    }

    fn read_block_impl(
        &self,
        index: usize,
        key: Option<u32>,
        stats: Option<&mut SectorStats>,
    ) -> Result<Vec<u8>, MpqError> {
        let block = *self
            .block_table
            .get(index)
            .ok_or(MpqError::BadBlockIndex(index))?;
        if !block.has(flags::EXISTS) {
            return Err(corrupt(format!("block {index} is not in use")));
        }
        if block.has(flags::PATCH_FILE) {
            return Err(MpqError::Unsupported(format!(
                "block {index} is a patch file"
            )));
        }
        if block.file_size == 0 {
            return Ok(Vec::new());
        }
        // An encrypted block needs its key; a plain block ignores any key given.
        let key = match (block.has(flags::ENCRYPTED), key) {
            (true, None) => return Err(MpqError::KeyRequired(index)),
            (true, Some(k)) => Some(k),
            (false, _) => None,
        };
        let data = self.read_raw(&block, index)?;
        let reader = BlockReader {
            index,
            block,
            key,
            sector_size: self.sector_size() as usize,
            stats,
        };

        if block.has(flags::SINGLE_UNIT) {
            return reader.single_unit(data);
        }
        let file_size = block.file_size as usize;
        let out = if block.is_compressed() {
            reader.compressed_sectors(data)?
        } else {
            reader.stored_sectors(data)?
        };
        if out.len() != file_size {
            return Err(corrupt(format!(
                "block {index}: decoded {} bytes, expected {file_size}",
                out.len()
            )));
        }
        Ok(out)
    }

    /// Recovers the key of an encrypted, compressed block without its name
    /// by trying candidate keys. Returns `None` if no candidate decodes.
    ///
    /// A compressed block starts with its sector offset table, encrypted with the file
    /// key minus one, and the first offset is always the table's own length, which is
    /// known from the block's size. That one known dword pins the key down to 256
    /// candidates (one per low byte that selects the cipher's table entry). A candidate
    /// is kept when it also gives a plausible second offset and the whole block then
    /// decodes.
    pub fn recover_key(&self, index: usize) -> Result<Option<u32>, MpqError> {
        let block = *self
            .block_table
            .get(index)
            .ok_or(MpqError::BadBlockIndex(index))?;
        if !block.has(flags::ENCRYPTED)
            || !block.is_compressed()
            || block.has(flags::SINGLE_UNIT)
            || block.file_size == 0
            || block.compressed_size < 8
        {
            return Ok(None);
        }
        let sector_size = self.sector_size();
        let sectors = block.file_size.div_ceil(sector_size);
        let table_len = (sectors + 1 + u32::from(block.has(flags::SECTOR_CRC))) * 4;

        let mut head = [0u8; 8];
        read_exact_at(
            &self.file,
            &mut head,
            self.header.offset + u64::from(block.file_pos),
        )?;
        // For the first dword, cipher = plain ^ (key + seed) with
        // seed = 0xEEEEEEEE + CRYPT_TABLE[0x400 + (key & 0xFF)].
        let key_plus_table = (le_u32(&head, 0) ^ table_len).wrapping_sub(0xEEEE_EEEE);

        for i in 0..0x100 {
            let table_key = key_plus_table.wrapping_sub(CRYPT_TABLE[0x400 + i]);
            let mut probe = head;
            decrypt(&mut probe, table_key);
            if le_u32(&probe, 0) == table_len && le_u32(&probe, 4) <= table_len + sector_size {
                let candidate = table_key.wrapping_add(1);
                if self.read_block(index, Some(candidate)).is_ok() {
                    return Ok(Some(candidate));
                }
            }
        }
        Ok(None)
    }

    /// File names from `(listfile)`, if the archive has one.
    pub fn listfile(&self) -> Result<Option<Vec<String>>, MpqError> {
        if !self.contains("(listfile)") {
            return Ok(None);
        }
        let bytes = self.read("(listfile)")?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(Some(
            text.split(['\r', '\n', ';'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
        ))
    }
}

/// What decoding one block needs, once its entry is checked and its bytes are read.
struct BlockReader<'a> {
    index: usize,
    block: BlockEntry,
    /// The file key, for an encrypted block.
    key: Option<u32>,
    sector_size: usize,
    stats: Option<&'a mut SectorStats>,
}

impl BlockReader<'_> {
    fn codec_error(&self, source: CodecError) -> MpqError {
        MpqError::Codec {
            block: self.index,
            source,
        }
    }

    fn short_block(&self) -> MpqError {
        corrupt(format!(
            "block {} is shorter than its file size",
            self.index
        ))
    }

    /// A file stored as one unit: decrypted whole with the file key, then decompressed
    /// as a single sector if it is stored smaller than the file.
    fn single_unit(mut self, mut data: Vec<u8>) -> Result<Vec<u8>, MpqError> {
        if let Some(key) = self.key {
            decrypt(&mut data, key);
        }
        let file_size = self.block.file_size as usize;
        if self.block.is_compressed() && data.len() < file_size {
            return decompress_sector(&self.block, &data, file_size, self.stats.as_deref_mut())
                .map_err(|source| self.codec_error(source));
        }
        if data.len() < file_size {
            return Err(self.short_block());
        }
        data.truncate(file_size);
        Ok(data)
    }

    /// A compressed file stored in sectors. The block starts with an offset table: one
    /// offset per sector plus the end offset (and one more when sector checksums follow).
    /// When encrypted, the table uses the file key minus one and sector `i` uses the file
    /// key plus `i`. A sector stored at its full size was not worth compressing and is
    /// copied as it is.
    fn compressed_sectors(mut self, mut data: Vec<u8>) -> Result<Vec<u8>, MpqError> {
        let index = self.index;
        let file_size = self.block.file_size as usize;
        let sector_size = self.sector_size;
        let sectors = file_size.div_ceil(sector_size);
        let entries = sectors + 1 + usize::from(self.block.has(flags::SECTOR_CRC));
        let table_len = entries * 4;
        if data.len() < table_len {
            return Err(corrupt(format!("block {index}: offset table truncated")));
        }
        let mut table = data[..table_len].to_vec();
        if let Some(key) = self.key {
            decrypt(&mut table, key.wrapping_sub(1));
        }
        let offsets: Vec<usize> = table
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c) as usize)
            .collect();
        if offsets[0] != table_len {
            return Err(corrupt(format!(
                "block {index}: offset table starts at {:#x}, expected {table_len:#x}",
                offsets[0]
            )));
        }
        if offsets.windows(2).any(|w| w[1] < w[0]) || offsets[sectors] > data.len() {
            return Err(corrupt(format!("block {index}: invalid sector offsets")));
        }

        // Grows with decoded sectors: `file_size` is untrusted.
        let mut out = Vec::new();
        for i in 0..sectors {
            let expected = sector_size.min(file_size - i * sector_size);
            let sector = &mut data[offsets[i]..offsets[i + 1]];
            if let Some(key) = self.key {
                decrypt(sector, key.wrapping_add(i as u32));
            }
            if sector.len() < expected {
                let bytes =
                    decompress_sector(&self.block, sector, expected, self.stats.as_deref_mut())
                        .map_err(|source| self.codec_error(source))?;
                out.extend_from_slice(&bytes);
            } else if sector.len() == expected {
                if let Some(s) = self.stats.as_deref_mut() {
                    s.masks[0] += 1;
                }
                out.extend_from_slice(sector);
            } else {
                return Err(corrupt(format!(
                    "block {index}: sector {i} is longer than its decompressed size"
                )));
            }
        }
        Ok(out)
    }

    /// An uncompressed file stored in sectors: no offset table, the sectors follow each
    /// other at full size, and sector `i` of an encrypted file uses the file key plus `i`.
    fn stored_sectors(self, mut data: Vec<u8>) -> Result<Vec<u8>, MpqError> {
        let file_size = self.block.file_size as usize;
        if data.len() < file_size {
            return Err(self.short_block());
        }
        data.truncate(file_size);
        if let Some(key) = self.key {
            for (i, chunk) in data.chunks_mut(self.sector_size).enumerate() {
                decrypt(chunk, key.wrapping_add(i as u32));
            }
        }
        if let Some(s) = self.stats {
            s.masks[0] += file_size.div_ceil(self.sector_size) as u64;
        }
        Ok(data)
    }
}

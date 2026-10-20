//! String tables (`.tbl`): the game's localised text, keyed by string id.
//!
//! A table is a header, an element-number index, an open-addressed hash table of 17-byte
//! slots, then NUL-terminated keys and values. [`StringTable::parse`] reads the whole
//! file into owned entries (values stay raw bytes), [`StringTable::element`] looks a
//! string up by element number and [`StringTable::get`] by key. The font table that
//! pairs with a glyph sheet lives in the `font` module, not here.
//!
//! Input is the player's own `.tbl` files from the game archives; the loader checks every
//! offset and size against the file so a damaged table is an error, never a panic.

use crate::cursor::{invalid, Cursor, FormatError};

const FORMAT: &str = "tbl";

/// The 21-byte header at the start of a `.tbl` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TblHeader {
    /// Checksum field, stored as read; not verified.
    pub crc: u16,
    /// Number of element numbers in the index array that follows the header.
    pub num_elements: u16,
    /// Number of 17-byte hash slots.
    pub hash_table_size: u32,
    /// Version byte, stored as read.
    pub version: u8,
    /// Offset where the string data starts, stored as read.
    pub data_start: u32,
    /// Most slots a key lookup probes before giving up.
    pub max_tries: u32,
    /// File size recorded in the header, stored as read.
    pub file_size: u32,
}

/// One 17-byte hash slot with its key and value strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TblEntry {
    /// The raw `used` byte. Any nonzero byte makes the parser read the strings; key
    /// lookup accepts only exactly 1 (see `probe_skips_unused_slots`).
    pub used: u8,
    /// Element number stored in the slot.
    pub index: u16,
    /// Key hash stored in the slot, as read.
    pub hash: u32,
    /// Key bytes (without the NUL). Empty for unused slots.
    pub key: Vec<u8>,
    /// Value bytes (without the NUL), kept as raw bytes and not decoded here. The
    /// LoD tables hold UTF-8 text, so the color-code lead `ÿ` is stored as
    /// `C3 BF`; callers decode when they load the tables.
    pub value: Vec<u8>,
}

/// A parsed string table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTable {
    /// The file header.
    pub header: TblHeader,
    /// Element number → hash slot.
    pub indices: Vec<u16>,
    /// Hash slots.
    pub entries: Vec<TblEntry>,
}

/// The key hash stored in and used to probe the table, before reduction modulo the table
/// size.
pub fn key_hash(key: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in key {
        // Each key byte is sign-extended: 0x80 adds 0xFFFFFF80
        // (`hash_vectors` pins this).
        h = (h << 4).wrapping_add(c as i8 as i32 as u32);
        let t = h & 0xF000_0000;
        if t != 0 {
            h = (h & 0x0FFF_FFFF) ^ (t >> 24);
        }
    }
    h
}

fn cstring(data: &[u8], at: usize) -> Result<&[u8], FormatError> {
    let tail = data
        .get(at..)
        .ok_or_else(|| invalid(FORMAT, format!("key offset {at:#x} past end of file")))?;
    let len = tail
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| invalid(FORMAT, format!("unterminated key at {at:#x}")))?;
    Ok(&tail[..len])
}

impl StringTable {
    /// Parses a whole `.tbl` file. Fails on truncation, an index outside the hash table,
    /// or a key or value offset past the end of the file.
    pub fn parse(data: &[u8]) -> Result<StringTable, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        let header = TblHeader {
            crc: c.u16()?,
            num_elements: c.u16()?,
            hash_table_size: c.u32()?,
            version: c.u8()?,
            data_start: c.u32()?,
            max_tries: c.u32()?,
            file_size: c.u32()?,
        };
        let indices = (0..header.num_elements)
            .map(|_| c.u16())
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(&bad) = indices
            .iter()
            .find(|&&i| u32::from(i) >= header.hash_table_size)
        {
            return Err(invalid(
                FORMAT,
                format!("index {bad} outside the hash table"),
            ));
        }
        // 17 bytes per slot; checked against the file before allocating.
        let table_bytes = u64::from(header.hash_table_size) * 17;
        if c.pos() as u64 + table_bytes > data.len() as u64 {
            return Err(invalid(FORMAT, "hash table extends past end of file"));
        }

        let mut entries = Vec::with_capacity(header.hash_table_size as usize);
        // Key and value bytes copied so far. Slots whose strings overlap
        // could otherwise copy the same bytes once per slot (quadratic in
        // the file size); strings that don't overlap never exceed it.
        let mut copied = 0usize;
        for slot in 0..header.hash_table_size {
            let used = c.u8()?;
            let index = c.u16()?;
            let hash = c.u32()?;
            let key_offset = c.u32()? as usize;
            let value_offset = c.u32()? as usize;
            let value_length = usize::from(c.u16()?);
            let (key, value) = if used != 0 {
                let key = cstring(data, key_offset)
                    .map_err(|e| invalid(FORMAT, format!("slot {slot}: {e}")))?;
                let value: &[u8] = if value_length == 0 {
                    &[]
                } else {
                    value_offset
                        .checked_add(value_length - 1)
                        .and_then(|end| data.get(value_offset..end))
                        .ok_or_else(|| {
                            invalid(FORMAT, format!("slot {slot}: value past end of file"))
                        })?
                };
                copied += key.len() + value.len();
                if copied > data.len() {
                    return Err(invalid(
                        FORMAT,
                        format!("slot {slot}: strings add up to more than the file size"),
                    ));
                }
                (key.to_vec(), value.to_vec())
            } else {
                (Vec::new(), Vec::new())
            };
            entries.push(TblEntry {
                used,
                index,
                hash,
                key,
                value,
            });
        }
        Ok(StringTable {
            header,
            indices,
            entries,
        })
    }

    /// The entry for element number `i`, or `None` if out of range or empty.
    pub fn element(&self, i: usize) -> Option<&TblEntry> {
        let slot = usize::from(*self.indices.get(i)?);
        self.entries.get(slot).filter(|e| e.used != 0)
    }

    /// Hash slot of `key`, by linear probing from `key_hash % size`, for at most
    /// `max_tries` slots.
    pub fn find_slot(&self, key: &[u8]) -> Option<usize> {
        let size = self.header.hash_table_size as usize;
        if size == 0 {
            return None;
        }
        let mut slot = key_hash(key) as usize % size;
        // Probing past `size` slots revisits slots already seen, so the
        // answer is the same with the count capped at the table size.
        let tries = (self.header.max_tries as usize).min(size);
        for _ in 0..tries {
            // A slot whose `used` byte is not exactly 1 is skipped; the
            // probe goes on (`probe_skips_unused_slots`).
            let e = &self.entries[slot];
            if e.used == 1 && e.key == key {
                return Some(slot);
            }
            slot = (slot + 1) % size;
        }
        None
    }

    /// The value for `key`.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.find_slot(key)
            .map(|s| self.entries[s].value.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_vectors() {
        assert_eq!(key_hash(b""), 0);
        assert_eq!(key_hash(b"A") % 1000, 65);
        assert_eq!(key_hash(b"AB") % 1000, 106);
        // signed key byte: 0x80 adds 0xFFFFFF80
        assert_eq!(key_hash(&[0x80]), 0x0FFF_FF70);
        assert_eq!(key_hash(&[0x80]) % 1000, 312);
    }

    #[test]
    fn probe_skips_unused_slots() {
        // "A" (home slot 1) moved to slot 2; slot 1 left unused.
        let mut t = StringTable::parse(&build(&[("A", "x")], 4)).unwrap();
        t.entries.swap(1, 2);
        t.indices[0] = 2;
        assert_eq!(t.find_slot(b"A"), Some(2));
        assert_eq!(t.get(b"A"), Some(&b"x"[..]));
        // a used byte other than 1 is skipped too
        t.entries[2].used = 2;
        assert_eq!(t.find_slot(b"A"), None);
        assert!(t.element(0).is_some());
    }

    /// Builds a table in the file layout: header,
    /// indices, slots, then key/value strings.
    fn build(pairs: &[(&str, &str)], size: u32) -> Vec<u8> {
        let header_len = 21 + 2 * pairs.len() + 17 * size as usize;
        let mut slots: Vec<Option<(usize, u32)>> = vec![None; size as usize];
        let mut indices = Vec::new();
        for (i, (k, _)) in pairs.iter().enumerate() {
            let h = key_hash(k.as_bytes());
            let mut s = (h % size) as usize;
            while slots[s].is_some() {
                s = (s + 1) % size as usize;
            }
            slots[s] = Some((i, h));
            indices.push(s as u16);
        }
        let mut strings = Vec::new();
        let mut offsets = Vec::new();
        for (k, v) in pairs {
            let ko = header_len + strings.len();
            strings.extend_from_slice(k.as_bytes());
            strings.push(0);
            let vo = header_len + strings.len();
            strings.extend_from_slice(v.as_bytes());
            strings.push(0);
            offsets.push((ko, vo, v.len() + 1));
        }
        let total = header_len + strings.len();
        let mut d = Vec::new();
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&(pairs.len() as u16).to_le_bytes());
        d.extend_from_slice(&size.to_le_bytes());
        d.push(1);
        d.extend_from_slice(&(header_len as u32).to_le_bytes());
        d.extend_from_slice(&size.to_le_bytes());
        d.extend_from_slice(&(total as u32).to_le_bytes());
        for i in &indices {
            d.extend_from_slice(&i.to_le_bytes());
        }
        for slot in &slots {
            match slot {
                Some((i, h)) => {
                    let (ko, vo, vl) = offsets[*i];
                    d.push(1);
                    d.extend_from_slice(&(*i as u16).to_le_bytes());
                    d.extend_from_slice(&h.to_le_bytes());
                    d.extend_from_slice(&(ko as u32).to_le_bytes());
                    d.extend_from_slice(&(vo as u32).to_le_bytes());
                    d.extend_from_slice(&(vl as u16).to_le_bytes());
                }
                None => d.extend_from_slice(&[0u8; 17]),
            }
        }
        d.extend(strings);
        d
    }

    #[test]
    fn round_trip() {
        // "A" and "Q" collide in a table of 16 (65 % 16 == 81 % 16 == 1).
        let pairs = [("A", "first"), ("Q", "second"), ("Key3", "ÿc1red")];
        let t = StringTable::parse(&build(&pairs, 16)).unwrap();
        for (i, (k, v)) in pairs.iter().enumerate() {
            let e = t.element(i).unwrap();
            assert_eq!(e.key, k.as_bytes());
            assert_eq!(e.value, v.as_bytes());
            assert_eq!(t.get(k.as_bytes()), Some(v.as_bytes()));
        }
        assert_eq!(t.get(b"missing"), None);
        assert!(t.element(3).is_none());
    }

    #[test]
    fn values_are_kept_as_bytes() {
        // The color-code lead `ÿ` as the game's tables store it (UTF-8 `C3 BF`), then
        // bytes that are not UTF-8: both come back unchanged, not decoded.
        let utf8 = [0xC3, 0xBF, b'c', b'1', b'x'];
        let raw = [0xFF, b'c', 0x80];
        let mut data = build(&[("A", "abcde"), ("B", "abc")], 4);
        let at = |d: &[u8], v: &[u8]| d.windows(v.len()).rposition(|w| w == v).unwrap();
        let (a, b) = (at(&data, b"abcde\0"), at(&data, b"abc\0"));
        data[a..a + 5].copy_from_slice(&utf8);
        data[b..b + 3].copy_from_slice(&raw);
        let t = StringTable::parse(&data).unwrap();
        assert_eq!(t.get(b"A"), Some(&utf8[..]));
        assert_eq!(t.get(b"B"), Some(&raw[..]));
        assert!(std::str::from_utf8(t.get(b"A").unwrap()).unwrap() == "ÿc1x");
    }

    #[test]
    fn truncated_is_an_error() {
        let data = build(&[("A", "x")], 4);
        assert!(StringTable::parse(&data[..data.len() - 4]).is_err());
        assert!(StringTable::parse(&data[..30]).is_err());
    }

    #[test]
    fn regress_lookup_with_huge_max_tries() {
        // A full table and max_tries u32::MAX: a missing key probed 4
        // billion slots. Probing past the table size revisits slots, so
        // the capped probe gives the same answer.
        let mut data = build(&[("A", "x"), ("B", "y"), ("C", "z")], 3);
        data[0x0D..0x11].copy_from_slice(&u32::MAX.to_le_bytes());
        let t = StringTable::parse(&data).unwrap();
        // Under the robustness deadline (the old probe took ~45 s here).
        let t = crate::robust::bounded(move || {
            assert_eq!(t.get(b"missing"), None);
            t
        });
        assert_eq!(t.get(b"C"), Some(&b"z"[..]));
    }

    #[test]
    fn regress_slots_sharing_one_string() {
        // 64 used slots whose keys all start at one 200-byte string copied
        // it 64 times (quadratic in the file size for large tables).
        let size = 64usize;
        let strings_at = 21 + 17 * size;
        let mut d = Vec::new();
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&(size as u32).to_le_bytes());
        d.push(1);
        d.extend_from_slice(&(strings_at as u32).to_le_bytes());
        d.extend_from_slice(&(size as u32).to_le_bytes());
        d.extend_from_slice(&((strings_at + 201) as u32).to_le_bytes());
        for _ in 0..size {
            d.push(1);
            d.extend_from_slice(&[0; 6]);
            d.extend_from_slice(&(strings_at as u32).to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
        }
        d.extend_from_slice(&[b'k'; 200]);
        d.push(0);
        let err = StringTable::parse(&d).unwrap_err();
        assert!(err.to_string().contains("more than the file size"), "{err}");
    }

    #[test]
    fn integers_are_little_endian() {
        let mut data = build(&[("A", "xyz")], 4);
        data[0..2].copy_from_slice(&[0x34, 0x12]);
        let t = StringTable::parse(&data).unwrap();
        assert_eq!(t.header.crc, 0x1234);
        assert_eq!(t.header.num_elements, 1);
        assert_eq!(t.header.hash_table_size, 4);
        assert_eq!(t.header.file_size as usize, data.len());
        let e = t.element(0).unwrap();
        assert_eq!(e.hash, key_hash(b"A"));
        assert_eq!(e.value, b"xyz");
        // The same bytes read big-endian would give 0x0400_0000 slots.
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 4);
    }

    #[test]
    fn crc_is_not_verified_and_magic_tells_fonts_apart() {
        let data = build(&[("A", "first"), ("Q", "second")], 8);
        let t = StringTable::parse(&data).unwrap();
        for crc in [0x0000u16, 0x1234, 0xFFFF] {
            let mut d = data.clone();
            d[0..2].copy_from_slice(&crc.to_le_bytes());
            let u = StringTable::parse(&d).unwrap();
            assert_eq!(u.entries, t.entries);
            assert_eq!(u.header.crc, crc);
        }
        // Classification by content: "Woo!" is a font table, not a string
        // table; a string table never starts with it here.
        assert!(!crate::font::FontTable::is_font_table(&data));
        let mut font = b"Woo!".to_vec();
        font.extend_from_slice(&[1, 0, 0, 0, 0, 0, 16, 12]);
        assert!(crate::font::FontTable::is_font_table(&font));
        assert!(crate::font::FontTable::parse(&font).is_ok());
    }

    mod robust {
        use super::*;
        use crate::robust::{bounded, bytes, mutated};
        use crate::robust_tests::config;
        use proptest::prelude::*;

        fn valid() -> Vec<u8> {
            build(&[("A", "first"), ("Q", "second"), ("Key3", "x")], 8)
        }

        #[test]
        fn builder_is_valid() {
            assert!(StringTable::parse(&valid()).is_ok());
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(data in mutated(valid()), key in bytes(8), i in any::<usize>()) {
                bounded(move || {
                    if let Ok(t) = StringTable::parse(&data) {
                        let _ = t.get(&key);
                        let _ = t.get(b"Q");
                        let _ = t.element(i);
                        let _ = t.element(i % 4);
                    }
                });
            }
        }
    }
}

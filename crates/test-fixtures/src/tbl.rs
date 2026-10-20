//! Writes string tables that [`d2_formats::tbl::StringTable`] reads: the
//! 21-byte header, the element indices, the hash slots placed by the
//! format's key hash and linear probe, then each key and value NUL-terminated.

use d2_formats::tbl::key_hash;

/// A `.tbl` file with `entries` as elements 0, 1, … (key, value).
/// Keys must be unique and non-empty.
pub fn write(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let n = entries.len();
    assert!(n <= usize::from(u16::MAX), "too many strings");
    let size = (n * 2).max(1) as u32;
    let mut slots: Vec<Option<usize>> = vec![None; size as usize];
    let mut indices = Vec::with_capacity(n);
    let mut max_tries = 1u32;
    for (i, (key, _)) in entries.iter().enumerate() {
        assert!(!key.is_empty(), "empty key");
        assert!(
            entries[..i].iter().all(|(k, _)| k != key),
            "duplicate key {:?}",
            String::from_utf8_lossy(key)
        );
        let mut slot = key_hash(key) % size;
        let mut tries = 1;
        while slots[slot as usize].is_some() {
            slot = (slot + 1) % size;
            tries += 1;
        }
        max_tries = max_tries.max(tries);
        slots[slot as usize] = Some(i);
        indices.push(slot as u16);
    }

    let data_start = 21 + n * 2 + size as usize * 17;
    let mut strings = Vec::new();
    let mut at = Vec::with_capacity(n); // (key offset, value offset, value length)
    for (key, value) in entries {
        let k = data_start + strings.len();
        strings.extend_from_slice(key);
        strings.push(0);
        let v = data_start + strings.len();
        strings.extend_from_slice(value);
        strings.push(0);
        at.push((k as u32, v as u32, (value.len() + 1) as u16));
    }
    let file_size = (data_start + strings.len()) as u32;

    let mut out = Vec::with_capacity(file_size as usize);
    out.extend_from_slice(&0u16.to_le_bytes()); // crc, not verified
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.push(1); // version
    out.extend_from_slice(&(data_start as u32).to_le_bytes());
    out.extend_from_slice(&max_tries.to_le_bytes());
    out.extend_from_slice(&file_size.to_le_bytes());
    for i in &indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    for slot in &slots {
        match *slot {
            None => out.extend_from_slice(&[0; 17]),
            Some(i) => {
                let (k, v, len) = at[i];
                out.push(1);
                out.extend_from_slice(&(i as u16).to_le_bytes());
                out.extend_from_slice(&key_hash(entries[i].0).to_le_bytes());
                out.extend_from_slice(&k.to_le_bytes());
                out.extend_from_slice(&v.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
        }
    }
    out.extend_from_slice(&strings);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::tbl::StringTable;

    #[test]
    fn round_trip() {
        let keys: Vec<String> = (0..300).map(|i| format!("key{i}")).collect();
        let entries: Vec<(&[u8], &[u8])> =
            keys.iter().map(|k| (k.as_bytes(), k.as_bytes())).collect();
        let t = StringTable::parse(&write(&entries)).unwrap();
        assert_eq!(t.header.num_elements, 300);
        assert_eq!(t.header.file_size as usize, write(&entries).len());
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(t.get(k.as_bytes()), Some(k.as_bytes()), "{k}");
            assert_eq!(t.element(i).unwrap().key, k.as_bytes());
        }
        assert_eq!(t.get(b"missing"), None);
        assert_eq!(t.get(b"KEY1"), None, "case-sensitive");
    }

    #[test]
    fn empty_value_and_empty_table() {
        let t = StringTable::parse(&write(&[(b"a", b"")])).unwrap();
        assert_eq!(t.get(b"a"), Some(&b""[..]));
        let t = StringTable::parse(&write(&[])).unwrap();
        assert_eq!(t.header.num_elements, 0);
    }
}
